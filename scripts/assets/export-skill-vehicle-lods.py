"""Export the three runtime LOD tiers of a fleet body from its vehicle-skill build.

    blender -b --factory-startup -P scripts/assets/export-skill-vehicle-lods.py -- \
        --body <key> --blend <body.blend> --cage <cage.json> --parts <parts.json> \
        --output <dir> [--work <dir>] [--promote]

Bodies: sedan, compactSuv, compactHatch, taxi, coupe, fullSizeSuv, wagon, minivan, pickup,
deliveryVan, boxTruck, cityBus, patrolSedan, exoticCoupe, and the trims taxiCheckered, taxiAdBoard, sedanSport, sedanLuxe,
compactHatchSport, compactHatchCross, patrolSuv (`_variant`: the parent's entry under the trim's slot). Check an export with
scripts/assets/check-skill-vehicle-glb.py.

One recipe serves every skill-built body; `BODIES` holds what differs between them (catalogue
slot, length, wheel radius, plate stations, and optional per-body keys). Optional keys change
only the body that carries them, so a body's output does not change when another is added:

* `doors` (two-door bodies), `windowDoors` (door glass on some doors only), `noTrunk` /
  `noRearWindow` (barn doors, no tailgate lid, no rear glass), `slideDoors` (a sliding door
  `SlideDoor_<c>` keeps its own pivot node on LOD0/LOD1, its skin is the `Panel_Slide<c>` zone
  on the cage tiers, and it carries slide extras the runtime bakes static today);
* `plateCard` (per-anchor runtime card size, metres, and optional `tilt: 'surface'`, written as the anchor node's
  plateW / plateH extras and rotation; anchors without it keep the runtime's default card; set per body in the
  "plate cards" block after the entries);
* `plateBlanks` / `plateTargets` (the build's own plate boxes give way to the runtime card;
  a plate may seat on a named bumper part); `plateProudM` (how far each plate anchor is set out
  from the surface behind it, 3 mm by default; a body whose anchor read back under the 3 mm
  contract from the GLB seats it at 3.5 mm);
* `bedCut` (an open cargo bed is cut out of the cage shell and lining at the mid and far tiers);
* `midDrop` / `midTargets` / `farTargets` / `farKeep` (per-body part rules at the cage tiers),
  `nearPool` / `nearTrimCand` / `nearPasses` (dense parts that share a near overage),
  `materials` (per-body material-name overrides), `lidStaticParts`
  (lid-mounted parts that ride on the body, out of the lid's draw pools), `articulatedFoldFinal` (a last fold on the
  door and lid materials, applied after the others), `lampZones` (faces of a lamp material inside a box re-assigned
  to another lamp material at export: indicator and stop-lamp channels for a build without them), `paintAs` (VehiclePaint ships under an untinted
  livery name and colour: the taxi body, which the runtime must not tint), `paintRename` (VehiclePaint renamed in
  the exported GLB only, its authored colour kept: the black patrol body), `tailLampNodes` (extra lamp nodes that
  are tail lamps on the cage tiers);
* `liningPaint` / `liningFacing` / `dropPokingLinings` / `thinSkip` (a deep door scallop: a livery counts as
  paint for the lining tests, more lining faces are ray-tested, inner-wall faces in front of the paint are
  dropped, and the named panels' inner walls are not thinned);
* `noLids` (no bonnet or boot lid skins), and for a body whose cage is only part of it (the box
  truck's cab) the reducer-path rules `midByName` / `farSkinTargets` / `farKeepExtra` /
  `farDrop` / `farDropSuffix` (a `farSkinTargets` key ending in `_` is a name prefix);
* `renames` (per-body node renames into the runtime vocabulary), `optionalNodes` (runtime nodes
  a body does not have: the bus has no pedals or front passenger seat), and for the bus's long
  flat-sided shell `midBodyPlanarDeg` / `midStripInner` / `protectZones` (mid/far reducer path:
  paint-zone outlines are held like boundaries by vehicle_mid_lod);
* the retrofit round (2026-10-04, the "retrofit round" block after the entries): `tintAsGlass` (VehicleGlassTint
  is glass for the decal skip, the greenhouse lining and the seam outline test) and `tierGlass` (darker, more opaque
  VehicleGlass / VehicleGlassTint at LOD1/LOD2 only, TIER_GLASS); `pokingMaterials` (inner-wall faces of more
  materials tested by drop_poking_linings, with a sampled pokes-through test); `planarOuterOnly` (the LOD0 planar
  merge only on outward-facing faces, seen from (0, 0, 0.9) or the key's own centre); `decalBoxClamp` (decal faces
  reaching outside the tier shell's box are dropped); `glassRefineExtra` (extra refine rounds on body-owned glass
  outlines at the named cage tier); the bus's `tierFritMerge` / `tierFritMax` (frit ribbons merged, or dropped past
  a triangle count, at the reducer tiers), `nearGlassSkip` (panes the near budget never halves), `nearReseatPlates`
  (plates reseated after the near budget fit), `farTyreSegments` (far tyres rebuilt as clean lathes) and
  `farFlatPrefixes` (far flat panels without an inner wall, flat-shaded); `farLampZones` (lampZones rules for LOD2
  only); `engineCoverTrunk` (an `EngineCover` node is the runtime's rear lid: the exotic coupe, not the bus);
  `exportFold` (articulated-part material folds applied as one tier is written, for the articulation draw budget);
  `nearKeepWhole` (node prefixes the near detail pass never reduces: the sedan family's steering rim);
  `exportDrop` (build part names removed with the stage parts before any tier: the bus's raked window bands).

The near tier (LOD0) path is the same for every body. Mid/far (cage tier) rules:

1. Arch (and bed) booleans run AFTER refine_zone_edges. The refine's shrinkwrap would otherwise
   pull the boolean's cut walls onto the outer skin as inward-facing triangles coincident with
   the flank (dark flank patches). Shell normals are rebuilt from the limit surface after the
   budget merge (`limit_normals`), and edges sharper than 50 deg are split.
2. A headlamp lens does not turn wholly into emissive VehicleHeadlamp. Only lens faces on the
   skin (within 12 mm of the limit surface, facing out) and inside the build's lamp outline
   (parts.json recess prism) are used, and each takes the material of what the near build
   shows through it (core, reflector, projector -> VehicleHeadlamp; DRL/indicator bars keep
   VehicleDRL/VehicleIndicator; housing -> dark trim).
3. Shut lines: at the mid tier a narrow dark seam ribbon (VehicleTrimBlack) is laid along each
   moving panel's outline from the near build's own panel edges, riding on that panel.

The source is a zstack /vehicle skill build: a hand-shaped, mirrored subdivision cage in
Blender, cut into panels and rigged there. No AI 3D generator is involved at any stage. The
.blend lives in the off-repo asset store; only its SHA-256 is recorded here.

Every tier is prepared from a fresh load of the source, so no reduction stacks on another:

* LOD0 keeps the skill build as reviewed, minus what the game must not or need not carry:
  the crush shape keys (the runtime deforms bodies itself; a GLB with hundreds of morph targets
  broke the hero bake), the hidden stage objects, and most of the INNER wall of the solidified
  shell. The paint surface, every panel edge and every seam are left exactly as built.
* LOD1 and LOD2 reduce part by part with the repo's own reducer
  (`scripts/assets/vehicle_mid_lod.py` `_reduce`, which protects open boundaries and verifies
  that no seam vertex moved), on an explicit per-part allocation for this assembly. Parts that
  cannot be seen at a tier's range (engine, pedals, calipers at the far tier) are dropped.

Contract fixes applied for the runtime (src/vehicles/hero-sedan-glb.js,
src/render/passes/vehicle-hero-overlay.js):

* the source is authored nose -Y, Z up, with the car's LEFT on +X, which is already the runtime
  frame after the glTF Y-up conversion; it is stamped `vehicleFrame: lhd-v1` WITHOUT a mirror
  (scripts/assets/vehicle_frame.py mirrors only builders that author in the mirrored frame);
* material names map to the engine vocabulary; `VehiclePaint` is the only tinted material;
* door hinge axes are rewritten into the glTF node frame the runtime reads, and the lids carry
  the `hingeAxis: 'X'` / `openLimitRadians` form `lidHinge` accepts;
* nodes the runtime binds by name (steering rim/hub/spokes, seat backs, floor, pedals, plates,
  wheel parts) are renamed to that vocabulary;
* the arch-lip beads are pressed out of the wheel wells (`seat_arch_lips`).

Wheel travel. The overlay lifts a wheel up to 0.11 m (MAX_WHEEL_LIFT_M) and clamps steer to the
limit it solves from this body. The front wells of the source build are fitted to that envelope
in the skill build itself (a game-resolution rebuild of the same cage: the lip-return ring held
inside the 4 mm wall and the lower arch corners widened fore/aft), not here, so the door-swing
proof the skill runs still covers them."""
import argparse
import hashlib
import json
import math
import struct
import sys
from pathlib import Path

import bmesh
import bpy
from mathutils import Matrix, Vector, geometry as mgeom
from mathutils.bvhtree import BVHTree

sys.path.insert(0, str(Path(__file__).resolve().parent))
from vehicle_mid_lod import _reduce as reduce_protected  # noqa: E402

BODIES = {
    'sedan': dict(label='Mid-size saloon (vehicle skill build)', lengthM=4.85, wheelRadiusM=0.34,
                  typeIds=['sedan'],
                  # plate stations (height in m): front on the bumper's black plate zone between grille and lower
                  # intake, at its centre (paintZones PlateFront z 0.400-0.515; 0.44 until the plate cards round
                  # put a card the zone's height there); rear in the bumper's plate recess
                  plates=(('PlateFront', 0.4575, -1), ('PlateRear', 0.6375, 1)),
                  # set at exactly 3 mm, the rear anchor read back 2.99 mm proud from the GLB
                  plateProudM=0.0035,
                  cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=1,
                                   decalOffset=0.004, decalBudget=1200),
                  cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, decalOffset=0.008, decalBudget=420)),
    'compactSuv': dict(label='Compact SUV (vehicle skill build)', lengthM=4.55, wheelRadiusM=0.36,
                       typeIds=['compactSuv'],
                       # centres of the build's own plate recesses (parts.json Plate_Front/Plate_Rear)
                       plates=(('PlateFront', 0.5975, -1), ('PlateRear', 0.805, 1)),
                       # this shell carries ~60% more outer faces than the sedan's (cladding and
                       # trim zones cut into it); merge only faces that are coplanar to 0.5 deg
                       planarDeg=0.5,
                       # rebuild the steering rim as a coarse true torus at the mid tier, so the
                       # runtime can still measure and turn it (a decimated rim is not circular)
                       midRim=(24, 6),
                       # mid and far are built from the hand-shaped cage (build_cage_tier), not
                       # decimated: mid = cage level 1 split into the moving panels, with a
                       # level-0 inner lining 20 mm behind it; far = cage level 0, flat
                       cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=1,
                                   decalOffset=0.004, decalBudget=1200),
                       cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, decalOffset=0.008, decalBudget=420)),
    # five-door compact hatch, top-hinged tailgate (the runtime's Trunk); plates: the front blank on
    # the intake (z 0.425), the rear recess high on the bumper (z 0.59)
    'compactHatch': dict(label='Compact five-door hatchback (vehicle skill build)', lengthM=4.06, wheelRadiusM=0.315,
                         # door/lid draw budget: the tailgate's dark cast and dark metal fittings join trim black
                         articulatedFold={'VehicleMetalCast': 'VehicleTrimBlack', 'VehicleMetalDark': 'VehicleTrimBlack'},
                         typeIds=['compactHatch'], nearWheelShare=0.45,
                         plates=(('PlateFront', 0.425, -1), ('PlateRear', 0.59, 1)),
                         planarDeg=0.5, midRim=(24, 6),
                         cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=2,
                                      decalOffset=0.004, decalBudget=1200),
                         cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, refineEdges=1, decalOffset=0.008, decalBudget=420)),
    # two long doors; cage-built mid and far tiers like the compactSuv
    'coupe': dict(label='Two-door sports coupe (vehicle skill build)', lengthM=4.50, wheelRadiusM=0.345,
                  typeIds=['coupe'], doors=('FL', 'FR'), nearWheelShare=0.45,
                  # front: the flat plate mount between grille and lower intake; rear: the plate recess
                  plates=(('PlateFront', 0.505, -1), ('PlateRear', 0.4675, 1)),
                  # the runtime card per anchor: the front pocket (parts.json Intake_Lower stem, z 0.452-0.563, 111 mm)
                  # takes a real 305 x 152 plate scaled to 0.72 (220 x 110 mm, 100 mm tall on the 24.4 deg mount rake,
                  # as tuner coupes run it), turned onto the mount; the rear pocket (Plate_Rear, 145 x 570 mm) the same
                  # plate scaled to the pocket less 5 mm, 281 x 140 (the "plate cards" block below)
                  plateCard={'front': dict(w=0.22, h=0.11, tilt='surface'), 'rear': dict(w=0.281, h=0.14)},
                  planarDeg=0.5, midRim=(24, 6),
                  cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=2,
                               decalOffset=0.004, decalBudget=1200),
                  cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, refineEdges=1, decalOffset=0.008, decalBudget=420)),
    # estate: four doors, top-hinged tailgate (the runtime's Trunk). Plates: the front on the black
    # plate zone of the bumper (parts.json paintZones PlateFront, z 0.43-0.542), the rear in the tailgate recess
    # (recesses Plate_Rear, z 0.618-0.744)
    'wagon': dict(label='Mid-size estate car (vehicle skill build)', lengthM=4.76, wheelRadiusM=0.335,
                  # door/lid draw budget: the rear-door underbody strips join trim black (the tailgate glass
                  # seal keeps its void black: folded, its outline visibly lightened)
                  articulatedFold={'VehicleUnderbody': 'VehicleTrimBlack'},
                  typeIds=['wagon'], nearWheelShare=0.45,
                  # the chrome trim sweeps (belt, sill, grille, bumper lines) and roof rails share a near overage
                  nearPool=('BeltChrome', 'SillChrome', 'GrilleBaseChrome', 'GrilleTopChrome', 'BumperLine', 'RearBumperSplit',
                            'DLOQuarterChrome', 'FrontLipChrome', 'DiffuserChrome', 'FogChrome', 'IntakeChrome', 'TailgateLine', 'RoofRail'),
                  plates=(('PlateFront', 0.486, -1), ('PlateRear', 0.681, 1)),
                  planarDeg=0.5, midRim=(24, 6),
                  cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=2,
                               decalOffset=0.004, decalBudget=1200),
                  cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, refineEdges=1, decalOffset=0.008, decalBudget=420)),
    # compact MPV: four doors, top-hinged tailgate. Plates: centres of the build's plate recesses
    # (Plate_Front z 0.436-0.568 -> 0.502, Plate_Rear z 0.672-0.852, box at 0.762); its own PlateFront/PlateRear boxes go
    'minivan': dict(label='Compact MPV (vehicle skill build)', lengthM=4.55, wheelRadiusM=0.36,
                    # door/lid draw budget: the tailgate's dark metal joins trim black; at the mid tier the door
                    # and lid chrome (handles, belt strips) is trim black too (part pools 33 -> 29 of 32)
                    articulatedFold={'VehicleMetalDark': 'VehicleTrimBlack'},
                    articulatedFoldMid={'VehicleChrome': 'VehicleTrimBlack'},
                    typeIds=['minivan'], nearWheelShare=0.45,
                    nearPool=('DoorStrip', 'RearBumperCrease', 'RearValenceSplit', 'TailgateCrease', 'ParkSensor', 'FogChrome',
                              'LowGrilleChrome', 'RoofRail'),
                    plates=(('PlateFront', 0.502, -1), ('PlateRear', 0.762, 1)), plateBlanks=('PlateFront', 'PlateRear'),
                    planarDeg=0.5, midRim=(24, 6),
                    cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=2,
                                 decalOffset=0.004, decalBudget=1200),
                    cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, refineEdges=1, decalOffset=0.008, decalBudget=420)),
    # full-size crew-cab pickup: four doors, bottom-hinged tailgate (the runtime's Trunk lid, X hinge),
    # open cargo bed. Plates: the front on the bumper face between the lower intake (z 0.59-0.648) and the grille housing
    # (z 0.925); the rear on the step bumper where the build's blank sits (z 0.59-0.74 -> 0.665)
    'pickup': dict(label='Full-size crew-cab pickup (vehicle skill build)', lengthM=5.89, wheelRadiusM=0.39,
                   typeIds=['pickup'], nearWheelShare=0.45,
                   plates=(('PlateFront', 0.79, -1), ('PlateRear', 0.665, 1)),
                   plateBlanks=('Bumper_PlateBlank',), plateTargets=('Bumper_Rear',),
                   planarDeg=0.5, midRim=(24, 6), bedCut=True,
                   nearPool=('ShoulderLine', 'LowerSculpt', 'Tread_', 'Flare_', 'Bumper_', 'Step', 'HeadBezel', 'Tailgate'),
                   # the all-terrain tread blocks sit inside the tyre's own envelope: the mid and far tyres carry the shape
                   # and the parts a mid-range view cannot resolve: step-bumper ribs, step pads and brackets, stake pockets,
                   # the chassis underside (frame, crossmembers, axle, driveshaft, muffler, tank, diff), hitch pin, tow hooks
                   midDrop=('Tread_', 'Bumper_StepRib', 'Step_Pad', 'Step_Bracket', 'Bed_StakePocket', 'Frame', 'Crossmember',
                            'Axle_', 'Driveshaft', 'Muffler', 'FuelTank', 'Diff_', 'HitchPin', 'TowHook', 'Exhaust_TipIn'),
                   midTargets={'Bed_Liner': (None, False), 'Bed_Hump': (None, False), 'Bed_Ribs': (60, False),
                               'Bed_RailCap': (60, False), 'Flare_': (160, True), 'Bumper_Rear': (160, True),
                               'Step_L': (48, False), 'Step_R': (48, False)},
                   farKeep=('Bed_Liner', 'Bed_Hump', 'Flare_', 'Bumper_Rear', 'Step_L', 'Step_R'),
                   farTargets={'Bed_Liner': (None, False), 'Bed_Hump': (24, False), 'Flare_': (48, False),
                               'Bumper_Rear': (48, False), 'Step_L': (16, False), 'Step_R': (16, False)},
                   cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=2,
                                decalOffset=0.004, decalBudget=1200),
                   cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, refineEdges=1, decalOffset=0.008, decalBudget=420)),
    # four-door notchback with a trunk lid; plates: the front blank on its flat mount over the lower grille
    # (z 0.46), the rear recess in the lid (z 0.815). The roof sign (TaxiSignBase / TaxiSign) rides every tier.
    'taxi': dict(label='Licensed taxi sedan (vehicle skill build)', lengthM=4.92, wheelRadiusM=0.345,
                 # the body is the taxi livery, not a tinted paint: VehiclePaint leaves as the untinted
                 # VehicleLiveryBodyYellow with the fleet's shipped yellow (the runtime tints only VehiclePaint)
                 paintAs=dict(name='VehicleLiveryBodyYellow', baseColor=(0.94, 0.62, 0.05, 1.0), metallic=0.2,
                              roughness=0.28),
                 typeIds=['taxi'], nearWheelShare=0.45,
                 plates=(('PlateFront', 0.46, -1), ('PlateRear', 0.815, 1)),
                 planarDeg=0.5, midRim=(24, 6),
                 cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=2,
                              decalOffset=0.004, decalBudget=1200),
                 cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, refineEdges=1, decalOffset=0.008, decalBudget=420)),
    # full-size three-row SUV at the real vehicle's footprint (5.70 m, wheelbase 3.30)
    'fullSizeSuv': dict(label='Full-size three-row SUV (vehicle skill build)', lengthM=5.70, wheelRadiusM=0.39,
                        # door/lid draw budget: the tailgate's dark metal joins trim black
                        articulatedFold={'VehicleMetalDark': 'VehicleTrimBlack'},
                        typeIds=['fullSizeSuv'],
                        plates=(('PlateFront', 0.585, -1), ('PlateRear', 1.1175, 1)),
                        plateProudM=0.0035,   # set at exactly 3 mm, the front anchor read back 2.97 mm proud
                        planarDeg=0.5, midRim=(24, 6),
                        # the smoked tail-lamp outline is opaque dark trim at runtime
                        materials={'Lens_Smoke': 'VehicleTrimBlack'},
                        # the build's own `PlateFront` blank (origin at the world origin) took the runtime name
                        # and pushed the anchor to `PlateFront.001`; it gives way to the runtime card on every tier
                        plateBlanks=('PlateFront',),
                        # this body carries ~10% more than the cap after glass: tyres, rims and the chrome trim tubes
                        nearTrimCand=('DLOChrome', 'GrilleFrame', 'BadgeBar', 'UpperChromeBar', 'UpperTierFrame',
                                      'LowerTierFrame', 'BumperChrome', 'Tread_', 'Step_'),
                        cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=1,
                                     decalOffset=0.004, decalBudget=1200),
                        cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, decalOffset=0.008, decalBudget=420)),
    # one-box panel van: hinged cab doors (FL/FR), rear barn doors on the rear-door slots (RL/RR, vertical
    # hinge axes in their own frame), a right-hand sliding cargo door (SlideDoor_R, static in the runtime today), a bonnet;
    # no tailgate lid, no rear glass. Plates: the front on the bumper face (Plate_Front z 0.586-0.698 -> 0.642), the rear
    # on the body-colour rear bumper moulding (RearBumper, z 0.475)
    'deliveryVan': dict(label='Full-size panel delivery van, medium roof (vehicle skill build)', lengthM=5.53, wheelRadiusM=0.375,
                        # one merged roof-lining vertex stood 246 mm above the roof (see _keep_inner_wall_inside)
                        innerWallBoxClamp=True,
                        typeIds=['deliveryVan'], nearWheelShare=0.45,
                        windowDoors=('FL', 'FR'), noTrunk=True, noRearWindow=True, slideDoors=('R',),
                        plates=(('PlateFront', 0.642, -1), ('PlateRear', 0.475, 1)),
                        plateBlanks=('PlateBlankFront', 'PlateBlankRear'), plateTargets=('RearBumper',),
                        planarDeg=0.5, midRim=(24, 6),
                        midDrop=('RearStepRib', 'CargoArch', 'CargoFloor'),
                        cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=2,
                                     decalOffset=0.004, decalBudget=1200),
                        cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, refineEdges=1, decalOffset=0.008, decalBudget=420)),
    # cab-over box truck: the cage is only the cab (the dry-van box, chassis and running gear are separate
    # parts from truck.py), so the mid and far tiers use the part-by-part reducer path, not the cage path. Four doors: the
    # cab doors and the rear barn doors (runtime Door_RL / Door_RR with hinge extras). Plates: the front blank on the
    # bumper (z 0.635), the rear on its bracket under the sill (z 0.92).
    'boxTruck': dict(label='Low-cab-forward box truck (vehicle skill build)', lengthM=7.62, wheelRadiusM=0.46,
                     # the part-by-part reducer swells the tyres at the mid and far tiers (drawn wheel 0.944 m
                     # against 0.920); every tyre vertex is held on or inside the rolling radius there
                     tyreRadiusClamp=True,
                     typeIds=['boxTruck'], nearWheelShare=0.40,
                     # front: the centre of the recess inside its surround (PlateRecessFront, z 0.575-0.735; 0.635 until
                     # the plate cards round put a US plate in it)
                     plates=(('PlateFront', 0.655, -1), ('PlateRear', 0.83, 1)), plateTargets=('BumperFront', 'RearStepBumper'),
                     plateBlanks=('PlateBlankFront', 'PlateBlankRear'), noLids=True, windowDoors=('FL', 'FR'), nearPasses=6,
                     # six tyres and rims (dual rear), the moulded sidewall detail and the rivet heads carry the overage
                     nearTrimCand=('Wheel_RL_', 'Wheel_RR_', 'Wheel_FL_', 'Wheel_FR_', 'Rivets', 'Drum_', 'Windscreen_Seal', 'RearWindow_Seal', 'Window_FL_Seal', 'Window_FR_Seal', 'Lamp_Cluster', 'Lamp_HeadU', 'Lamp_HeadL', 'Mirror'),
                     midDrop=('Rivets', 'Seam', 'RailReflector', 'CamKeeper', 'Door_RL_Rod', 'Door_RR_Rod', 'Door_RL_Hinge',
                              'Door_RR_Hinge', 'Door_RL_Handle', 'Door_RR_Handle', 'Door_RL_Rib', 'Door_RR_Rib', 'Door_RL_Inner',
                              'Door_RR_Inner', 'Cargo', 'ETrack', 'ScuffBand', 'Crossmember', 'BoxCrossmember', 'BoxRunner',
                              'FrontSpring', 'RearSpring', 'Shackle', 'Driveshaft', 'RearDiff', 'FrontAxle', 'RearAxleHousing',
                              'ExhaustStrap', 'ExhaustHanger', 'ExhaustShield', 'FuelStrap', 'FuelCap', 'DEFStrap', 'DEFCap',
                              'CabStepRib', 'CabStepHanger', 'MudFlapRib', 'MudFlapBar', 'RearStepRib', 'RearStepBracket',
                              'DockBumper', 'Lamp_BumperFog', 'Lamp_SideMark', 'Lamp_Clear', 'Lamp_ID', 'Tyre_', 'Wheel_RL_TyreSide',
                              'Wheel_RR_TyreSide', 'Wheel_RL_TyreInnerSide', 'Wheel_RR_TyreInnerSide', 'Drum_', 'Lamp_RoofMarkerBase',
                              'Mirror_FL_Foot', 'Mirror_FR_Foot', 'MirrorFL_Foot', 'MirrorFR_Foot', 'MirrorFL_Clamp', 'MirrorFR_Clamp',
                              'Wiper', 'Lamp_DoorSideRim', 'DoorHandle', 'BumperIntakeBar', 'PlateRecess', 'PlateBracket',
                              'Lamp_HeadUL_Reflector', 'Lamp_HeadUR_Reflector', 'Lamp_HeadUL_Bezel', 'Lamp_HeadUR_Bezel',
                              'Lamp_HeadUL_Marker', 'Lamp_HeadUR_Marker', 'Lamp_HeadUL_DRL', 'Lamp_HeadUR_DRL', 'Lamp_HeadLL_Reflector',
                              'Lamp_HeadLR_Reflector', 'Lamp_TailL_Bezel', 'Lamp_TailR_Bezel', 'Lamp_TurnL_Bezel', 'Lamp_TurnR_Bezel',
                              'Lamp_ReverseL_Bezel', 'Lamp_ReverseR_Bezel', 'Lamp_TailL_Reflector', 'Lamp_TailR_Reflector',
                              'Lamp_TurnL_Reflector', 'Lamp_TurnR_Reflector', 'Lamp_ReverseL_Reflector', 'Lamp_ReverseR_Reflector',
                              'TailPipe', 'DEFTank', 'CamHandle', 'CamPocket', 'Lamp_HeadUL_Shroud', 'Lamp_HeadUR_Shroud',
                              'Lamp_HeadUL_ProjLip', 'Lamp_HeadUR_ProjLip', 'Lamp_HeadUL_Back', 'Lamp_HeadUR_Back',
                              'Lamp_HeadLL_Back', 'Lamp_HeadLR_Back'),   # cam handles, projector detail
                     midByName={'Box_Shell': (1400, True), 'BumperFront': (300, True), 'FrontPostL': (48, False),
                                'FrontPostR': (48, False), 'RearStepBumper': (60, False), 'ArchGuardL': (60, False),
                                'ArchGuardR': (60, False), 'BadgeBand_Chrome': (48, False), 'FrameRailL': (24, False),
                                'FrameRailR': (24, False), 'MudFlapFL': (24, False), 'MudFlapFR': (24, False),
                                'MudFlapRL': (24, False), 'MudFlapRR': (24, False), 'ExhaustCan': (48, False),
                                'Wheel_RL_TyreInner': (160, False), 'Wheel_RR_TyreInner': (160, False),
                                'Wheel_RL_LipInner': (60, False), 'Wheel_RR_LipInner': (60, False)},
                     farSkinTargets={'Box_Shell': 260, 'BumperFront': 80, 'RearStepBumper': 24, 'FrontPostL': 16,
                                     'FrontPostR': 16, 'ExhaustCan': 24, 'FuelTank': 16, 'ArchGuardL': 16, 'ArchGuardR': 16},
                     farKeepExtra=('Box_Shell', 'TopRail', 'BottomRail', 'RearPost', 'RearHeader', 'RearSill', 'FrontPost',
                                   'RearCornerCasting', 'BumperFront', 'BumperStep', 'FrameRail', 'FuelTank', 'BatteryBox',
                                   'ExhaustCan', 'DEFTank', 'MudFlapF', 'MudFlapR', 'RearStepBumper', 'CabStep', 'Mirror',
                                   'ArchGuard', 'BadgeBand', 'Glass_Back'),
                     farDrop=('Window_FL_Seal', 'Window_FR_Seal', 'Windscreen_Seal', 'RearWindow_Seal', 'MirrorFL_Spot',
                              'MirrorFR_Spot', 'MirrorFL_Clamp', 'MirrorFR_Clamp', 'MirrorFL_Foot', 'MirrorFR_Foot', 'MirrorFL_Glass',
                              'MirrorFR_Glass', 'Lamp_Cluster', 'Lamp_SideMark', 'Lamp_ID', 'Lamp_Clear', 'Lamp_BumperFog',
                              'Lamp_DoorSide', 'Lamp_RoofMarkerBase', 'CabStep', 'BatteryBox', 'DEFTank', 'BumperStep',
                              'CamHandle', 'CamPocket', 'CamKeeperF', 'Lamp_HeadLL_Reflector', 'Lamp_HeadLR_Reflector'),
                     farDropSuffix=('_Bezel', '_Reflector', '_Marker', '_DRL', '_Shroud', '_ProjLip', '_Back', '_Divider'),
                     materials={'Rim': 'VehicleWheelSteel'},
                     nearPool=('Rivets', 'Seam', 'RailReflector', 'Door_RL_', 'Door_RR_', 'CamKeeper', 'MudFlap', 'Exhaust',
                               'Tyre_', 'Drum_', 'Lamp_')),
    # 40 ft low-floor transit bus. Doors on the right: the front door's two leaves are the runtime Door_FR /
    # Door_RR (conventional, vertical posts); the rear door leaves (DoorLeaf_RearA/B), the front ramp (Ramp_Front), the
    # engine lid and the two rear access doors are extra articulated parts recorded for integration. The bus has no
    # pedals or front passenger seat: those runtime nodes are optional here. Interior dropped at LOD1/LOD2.
    'cityBus': dict(label='40 ft low-floor city bus (vehicle skill build)', lengthM=12.50, wheelRadiusM=0.48,
                    typeIds=['cityBus'], planarDeg=0.4, nearWheelShare=0.35, midBodyPlanarDeg=None, farShellPlanarDeg=None,   # a planar merge before the collapse tented the roof (3/8 and 0.6/1.2 deg tried)
                    protectZones=True, midStripInner=True,
                    doors=('FR', 'RR'), windowDoors=('FR', 'RR'), noLids=True, nearPasses=6,
                    plates=(('PlateFront', 0.575, -1), ('PlateRear', 0.55, 1)),
                    plateBlanks=('PlateFront', 'PlateRear'),
                    plateTargets=('Bumper_Front', 'Bumper_Rear', 'PlateFront_Recess', 'PlateRear_Recess'),
                    renames={'DashConsole': 'Dash', 'DriverSeat_Base': 'Seat_FL', 'DriverSeat_Back': 'Seat_FL_Back',
                             'Floor_Raised': 'FloorPan'},
                    optionalNodes=('Seat_FR', 'PedalBrake', 'PedalThrottle'),
                    nearTrimCand=('Wheel_RL_', 'Wheel_RR_', 'Wheel_FL_', 'Wheel_FR_', 'Drum_', 'Lamp_Tail', 'Lamp_Stop',
                                  'Lamp_Turn', 'Lamp_Rev', 'Stanchion', 'GrabRail', 'Seat'),
                    nearPool=('Stanchion', 'GrabRail', 'RoofPod', 'Bumper', 'DestSign', 'Wiper', 'ArchLip', 'Lamp_',
                              'SkirtPanel', 'DoorLeaf_', 'AccessDoor_', 'EngineCover'),
                    midDrop=('Seat', 'Stanchion', 'GrabRail', 'Floor_', 'Headliner', 'LightStrip', 'RearBench', 'RearBulkhead',
                             'WheelHousing', 'DriverBarrier', 'FareBox', 'DashCluster', 'SteeringColumn', 'Ramp_Plate_',
                             'EngineCover_Louvre', 'EngineCover_Hinge', 'EngineCover_Latch', 'AccessDoor_L_Slat',
                             'RoofPod_Grille', 'Wiper', 'BikeRack', 'Bumper_Intake', 'CapSeam', 'Bumper_ShutLine',
                             'Lamp_SideMarker', 'Lamp_CapMarker', 'AxleRear_Diff', 'Axle', 'Glass_DestSign_Seal',
                             'GapFlange'),
                    midByName={'DestSign_LED': (None, False), 'DestSign_Board': (60, False), 'Bumper_Front': (160, True),
                               'Bumper_Rear': (120, True), 'RoofPod_Fairing': (120, True), 'EngineCover_Skin': (80, True),
                               'Windscreen': (160, True), 'Glass_DestSign': (40, True), 'Wheel_RL_TyreInner': (128, False),
                               'Wheel_RR_TyreInner': (128, False)},
                    farSkinTargets={'Bumper_Front': 40, 'Bumper_Rear': 32, 'RoofPod_Fairing': 24, 'EngineCover_Skin': 24,
                                    'DestSign_Board': 8, 'SkirtPanel_': 40, 'DoorLeaf_': 40, 'AccessDoor_': 24},
                    farKeepExtra=('Bumper_Front', 'Bumper_Rear', 'DestSign_Board', 'Glass_Side', 'Glass_Driver',
                                  'Glass_DestSign', 'Mirror', 'RoofPod_Fairing', 'EngineCover_Skin', 'DoorLeaf_',
                                  'AccessDoor_L_Skin', 'AccessDoor_R_Skin', 'SkirtPanel_', 'Wheel_RL_TyreInner',
                                  'Wheel_RR_TyreInner', 'Lamp_Tail', 'Lamp_Stop', 'Lamp_Turn', 'Lamp_HeadL_Lens',
                                  'Lamp_HeadR_Lens', 'PlateFront_Recess', 'PlateRear_Recess'),   # plate seats (rear card floated 10 mm)
                    farDrop=('Glass_Windscreen_Seal', 'Windscreen_Seal', 'RearWindow_Seal', 'Window_FR_Seal', 'Window_RR_Seal',
                             'Window_RearA_Seal', 'Window_RearB_Seal', 'Glass_DestSign_Seal', 'Lamp_SideMarker',
                             'Lamp_CapMarker', 'ArchLip', 'Drum_', 'Mirror_'),
                    farDropSuffix=('_Bezel', '_Bowl0', '_Bowl1', '_DRL', '_Liner', '_Back', '_Surround', '_Turn', '_Bowl',
                                   '_Base'),   # emitters kept: the far tier needs its VehicleHeadlamp channel
                    materials={'Rim': 'VehicleWheelSteel', 'GlassTint': 'VehicleGlassTint', 'Sign_LED': 'VehicleSignLED',
                               'Pole_Yellow': 'VehiclePole', 'Seat_Fabric': 'VehicleSeatFabric', 'Floor': 'VehicleFloor'}),
}
def _variant(parent, vid, label, mid_keep=(), far_rules=None, mid_rules=None, fold=None):
    """A catalogue trim on a skill body: the parent's entry under the variant's own catalogue slot.

    The trim parts (roof sign, livery strips, rails, spoiler, splitter) are kept whole at the mid tier and kept,
    with a small per-part budget, at the far tier. The parent's own keys (plates, folds, paintAs) carry over;
    `fold` adds trim-specific door/lid material folds for the articulation draw budget.
    """
    e = dict(BODIES[parent])
    e.update(label=label, typeIds=[vid])
    if fold:
        e['articulatedFold'] = {**(e.get('articulatedFold') or {}), **fold}
    far_rules = far_rules or {}
    e['midTargets'] = {**(e.get('midTargets') or {}), **{k: (v, True) for k, v in (mid_rules or {}).items()},
                       **{k: (None, False) for k in mid_keep}}
    e['farKeep'] = tuple(e.get('farKeep', ())) + tuple(far_rules)
    e['farTargets'] = {**(e.get('farTargets') or {}), **{k: (v, False) for k, v in far_rules.items()}}
    return e


BODIES.update({
    'taxiCheckered': _variant('taxi', 'taxiCheckered', 'Licensed taxi sedan, checker trim (trim of the taxi skill build)',
                              mid_keep=('Livery', 'TaxiSign'), far_rules={'Livery': None},
                              # draw budget: the tailgate's dark metal and the near-black checker squares (0.020
                              # against trim black's 0.012) join trim black on the doors and lids
                              fold={'VehicleMetalDark': 'VehicleTrimBlack', 'VehicleLiveryDark': 'VehicleTrimBlack'}),
    'taxiAdBoard': _variant('taxi', 'taxiAdBoard', 'Licensed taxi sedan, roof-board trim (trim of the taxi skill build)',
                            mid_keep=('Livery', 'TaxiSign', 'AdBoard'), far_rules={'AdBoard': 12, 'Livery': None},
                            fold={'VehicleMetalDark': 'VehicleTrimBlack'}),   # draw budget: the tailgate's dark metal
    'sedanSport': _variant('sedan', 'sedanSport', 'Mid-size saloon, sport trim (trim of the sedan skill build)',
                           mid_keep=('DeckSpoiler', 'SportSplitter', 'SportDiffuser', 'SportSkirt'),
                           far_rules={'DeckSpoiler': 12, 'SportSplitter': 12}),
    'sedanLuxe': _variant('sedan', 'sedanLuxe', 'Mid-size saloon, luxury trim (trim of the sedan skill build)',
                          mid_keep=('LuxeGrille', 'LuxeChrome'), far_rules={}),
    'compactHatchSport': _variant('compactHatch', 'compactHatchSport',
                                  'Compact five-door hatchback, sport trim (trim of the compactHatch skill build)',
                                  mid_keep=('HatchSpoiler', 'SportSplitter', 'SportSkirt', 'SportDiffuser'),
                                  far_rules={'HatchSpoiler': 12}),
    'compactHatchCross': _variant('compactHatch', 'compactHatchCross',
                                  'Compact five-door hatchback, raised trim (trim of the compactHatch skill build)',
                                  mid_keep=('SkidPlate',), mid_rules={'ArchCladdingEdge': 120, 'RoofRail': 160},
                                  far_rules={'RoofRail': 12, 'ArchCladding': 16, 'SillCladding': 12}),
})
# draw budget (shut batch 9 -> 8, part pools 33 -> 32): the clear reverse-strip lenses low on the tailgate ride on
# the body (lidStaticParts), so VehicleHeadlampLens leaves the lid. Folding the lens into glass, trim black or chrome
# visibly changed the strip; the door cladding's satin into trim black showed along the sills.
BODIES['compactHatchCross']['lidStaticParts'] = ('Lamp_TailGateSmoke_',)

# compact patrol SUV: a trim of the compactSuv skill body (light bar, push bar, A-pillar spotlight, door shields, roof
# antennas), kept whole at mid and on small budgets at far. The body is the patrol livery, not a tinted paint: the
# parent's VehiclePaint leaves as the untinted VehicleLiveryBodyWhite in the patrol sedan's white (`paintAs`; the
# runtime tints only VehiclePaint), beside the build's own VehicleLiveryBodyBlack zones.
BODIES['patrolSuv'] = _variant('compactSuv', 'patrolSuv', 'Compact patrol SUV (trim of the compactSuv skill build)',
                               mid_keep=('EmergencyBar', 'PushBar', 'Spotlight', 'DoorShield', 'RoofAntenna'),
                               far_rules={'EmergencyBar': 12, 'PushBarUpright': 12, 'PushBarCross': 16})
BODIES['patrolSuv']['paintAs'] = dict(name='VehicleLiveryBodyWhite', baseColor=(0.90, 0.90, 0.89, 1.0), metallic=0.0,
                                      roughness=0.30)
# draw budget (near part pools 35 -> 29, shut batch 9 -> 8; mid 35 -> 29): the two-tone livery puts a black AND a white
# body material on every door and lid, and the front doors carry the yellow shield, three part draws over the parent.
# On the doors and lids only, trim black (and everything the panel fold sends to it: door card, recess cups, lamp
# cups) joins the livery black: the same 0.012 black, 0.30 against 0.35 roughness, with the livery's clear coat.
BODIES['patrolSuv']['articulatedFoldFinal'] = {'VehicleTrimBlack': 'VehicleLiveryBodyBlack'}

# full-size rear-drive patrol sedan (5.05 m, wheelbase 3.05, 0.36 m steel wheels), four doors and a trunk lid like the
# taxi; cage-built mid and far tiers. Plates: the front blank on the bumper face (z 0.40), the rear on the blank in the
# bumper's plate recess (z 0.53); the build's own blanks give way to the runtime card. Police kit: the light bar is kept
# at every tier (whole at mid, its outer pieces on box budgets at far); push bar, spotlight and antennas on small budgets.
# The black body leaves as VehicleLiveryBodyBlack (`paintRename`, untinted), the white door livery keeps
# VehicleLiveryWhite and the dark rear glass VehicleGlassTint.
BODIES['patrolSedan'] = dict(
    label='Full-size patrol sedan (vehicle skill build)', lengthM=5.05, wheelRadiusM=0.36,
    typeIds=['patrolSedan'], nearWheelShare=0.45, plateProudM=0.0035,
    # the deep door scallop: the livery is paint for the lining tests, every lining face not facing with the paint near
    # it is ray-tested, the door panels' inner walls are not thinned (their chords bridged in front of the paint), and
    # an inner-wall face still standing in front of the paint is dropped
    liningPaint=('VehicleLiveryWhite', 'VehicleLiveryBodyBlack'), liningFacing=0.3, dropPokingLinings=True,
    thinSkip=('Panel_FL', 'Panel_FR', 'Panel_RL', 'Panel_RR'),
    # rear: on the build's own plate blank in the recess (PlateBlankRear z 0.454-0.606 in Plate_Rear_Housing z 0.445-0.640;
    # 0.5825 until the plate cards round put a US plate there)
    plates=(('PlateFront', 0.40, -1), ('PlateRear', 0.53, 1)), plateBlanks=('PlateBlankFront', 'PlateBlankRear'),
    planarDeg=0.5, midRim=(24, 6),
    materials={'GlassTint': 'VehicleGlassTint', 'Emerg_Red': 'VehicleEmergencyRed', 'Emerg_Blue': 'VehicleEmergencyBlue'},
    paintRename='VehicleLiveryBodyBlack',
    # the racetrack tail band's centre section on the trunk lid is a tail lamp on the cage tiers
    tailLampNodes=('Lamp_TailLid',),
    # mid: every light-bar piece on a 48-triangle box budget (the bar stays whole); the rest of the kit on small
    # budgets; small fittings dropped
    midTargets={'EmergencyBar': (48, False), 'Diffuser_Housing': (40, False), 'ExhaustCut': (24, False),
                'Glass_QuarterDoor': (72, True), 'LightBarFoot': (12, False), 'LightBarStrap': (12, False),
                'PushBar': (40, False), 'SpotlightHousing': (32, False), 'SpotlightBezel': (24, False),
                'SpotlightLens': (16, False), 'SpotlightShaft': (16, False), 'Antenna': (12, False)},
    midDrop=('SpotlightReflector', 'SpotlightCollar', 'SpotlightHandle', 'AntennaPuck', 'DiffuserFin', 'MarkerFront',
             'MarkerRear', 'FuelFlap', 'Partition'),
    # far: the light bar's base, spine and red / blue / clear lenses on box budgets, so the bar keeps its outline; its
    # takedown modules and ribs go with the far tier's other small parts
    farKeep=('EmergencyBarBase', 'EmergencyBarSpine', 'EmergencyBarRed', 'EmergencyBarBlue', 'EmergencyBarClear',
             'PushBarUpright', 'PushBarCross', 'SpotlightHousing'),
    farTargets={'EmergencyBar': (24, False), 'PushBarUpright': (12, False), 'PushBarCross': (16, False),
                'SpotlightHousing': (12, False)},
    cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=2,
                 decalOffset=0.004, decalBudget=1200),
    cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, refineEdges=1, decalOffset=0.008, decalBudget=420))

# [exoticCoupe run] mid-engine two-door coupe (4.39 m, wheelbase 2.62, staggered 0.3275 / 0.3455 m wheels), two doors, a
# rear-hinged bonnet and a front-hinged engine cover (renamed Trunk); cage-built mid and far tiers. Plates (3.5 mm proud):
# the front on the body-colour nose band over the centre intake (z 0.30), the rear centred on the dark diffuser face
# between the exhaust pairs (z 0.4975). The diffuser ribs and louvres go at mid; the far tier keeps the shell.
BODIES['exoticCoupe'] = dict(
    label='Mid-engine sports coupe (vehicle skill build)', lengthM=4.39, wheelRadiusM=0.3275,
    typeIds=['exoticCoupe'], doors=('FL', 'FR'), nearWheelShare=0.45, plateProudM=0.0035, optionalNodes=('RearWindow',),
    engineCoverTrunk=True,
    # mid/far rear and side features (cage tiers only; the near tier is unchanged): honeycomb walls not redrawn as decals,
    # tail lamps held through the decal collapse, the diffuser recess housing gives way to the shell (its zone and decal
    # stay), exhaust tips moved out to the shell, shut-line ribbons only along the panel border
    decalSkipSuffix=('_Grille',), lampKeepExtra=('VehicleTaillamp',), shellReplacesExtra=('Diffuser_Face',),
    tipsToShell=('Exhaust_',), seamOutlineOnly=True, reseatDecals=True,
    midTargets={'Exhaust_': (48, False)}, farKeep=('Exhaust_',), farTargets={'Exhaust_': (16, False)},
    plates=(('PlateFront', 0.30, -1), ('PlateRear', 0.4975, 1)),
    planarDeg=0.5, midRim=(24, 6),
    midDrop=('DiffuserFin', 'Louvre_'),
    # decal budgets doubled-ish: the rear band backing, diffuser face, louvre bay, intakes and lamps share one Body_Trim
    # (~4,400 source faces) and tore at 1,200 / 420; the tier cap is then met by the shell's planar merge
    cageMid=dict(level=1, lining=0.02, articulated=True, trimSides=5, trimStep=0.06, refineEdges=2,
                 decalOffset=0.004, decalBudget=2400),
    cageFar=dict(level=1, trimSides=4, trimStep=0.12, glassLining=0.03, refineEdges=1, decalOffset=0.008, decalBudget=700),
    # lamp channels the build does not carry (`lampZones`, faces re-assigned at export, no vertex moves): each tail
    # lamp's inboard end is the stop lamp, its outboard end the rear indicator, the middle stays the tail lamp; the
    # outboard tip of each head-lamp housing is the front indicator
    lampZones=(('VehicleBrakeLamp', ('VehicleTaillamp',), (0.45, 0.66), (1.85, 2.25), (0.66, 0.76)),
               ('VehicleIndicator', ('VehicleTaillamp',), (0.77, 0.90), (1.85, 2.25), (0.66, 0.76)),
               ('VehicleIndicator', ('VehicleTrimBlack',), (0.80, 0.90), (-1.75, -1.55), (0.34, 0.57))))

# ---------------------------------------------------------------------------------------------- retrofit round
# 2026-10-04 retrofit of the skill fleet (glass edges and frit, seam flanges, clear lamp lenses). Every retrofitted
# body exports its dark rear glass as its own VehicleGlassTint (merged into VehicleGlass it lost its tint) and seats
# its plate anchors 3.5 mm proud. The bodies re-exported through the shared retrofit recipe also treat that tinted
# glass as glass at the cage tiers (`tintAsGlass`) and leave darker, more opaque glass at LOD1/LOD2 (`tierGlass`: no
# cabin is drawn there, so the near tier's light glass read as pale panes). The compactHatch and coupe runs exported
# without those two. exoticCoupe was not retrofitted and keeps its entry as it is.
for _b in ('compactHatch', 'coupe', 'sedan', 'taxi', 'wagon', 'minivan', 'pickup', 'compactSuv', 'fullSizeSuv',
           'deliveryVan', 'boxTruck', 'cityBus', 'patrolSedan', 'patrolSuv', 'taxiCheckered', 'taxiAdBoard',
           'sedanSport', 'sedanLuxe', 'compactHatchSport', 'compactHatchCross'):
    BODIES[_b]['materials'] = {**(BODIES[_b].get('materials') or {}), 'GlassTint': 'VehicleGlassTint'}
    BODIES[_b].setdefault('plateProudM', 0.0035)
    if _b not in ('compactHatch', 'coupe'):
        BODIES[_b].update(tintAsGlass=True, tierGlass=True)
# coupe: the patrol sedan's lining keys (deep door scallop: more lining faces ray-tested, inner-wall chords in front
# of the paint dropped, the long doors' inner walls not thinned) and the exotic coupe's cage-tier keys (honeycomb walls
# not redrawn as decals, tail lamps held through the decal collapse, shut-line ribbons only along panel borders,
# decals reseated on the final shell), plus one extra far-tier refine round on the fixed glass outlines
BODIES['coupe'].update(liningFacing=0.3, dropPokingLinings=True, thinSkip=('Panel_FL', 'Panel_FR'),
                       decalSkipSuffix=('_Grille',), lampKeepExtra=('VehicleTaillamp',), seamOutlineOnly=True,
                       reseatDecals=True, glassRefineExtra={'cageFar': 1})
# wagon, fullSizeSuv: with the 0.2 mm edge roll a thinned inner-wall chord stood in front of the front door's paint
# under the mirror (a dark triangle on LOD0); on an articulated door that inner wall is already trim black
BODIES['wagon'].update(dropPokingLinings=True, pokingMaterials=('VehicleInterior', 'VehicleTrimBlack'))
BODIES['fullSizeSuv'].update(dropPokingLinings=True, pokingMaterials=('VehicleInterior', 'VehicleTrimBlack'),
                             planarOuterOnly=True)   # LOD0: no paint sheet across the lower grille
BODIES['pickup']['decalSkipSuffix'] = ('Grille_Front_Grille',)   # honeycomb walls not redrawn as mid/far decals
BODIES['deliveryVan']['decalBoxClamp'] = 0.02   # LOD2: no 1.9 m trim-black wedge under the front bumper
# cityBus: frit ribbons merged (and capped) at the mid/far tiers, LOD0 door panes left whole under their frit, plates
# reseated after the near budget fit, far tyres rebuilt as 18-sided lathes, far flat panels stripped and flat-shaded,
# LOD0 planar merge on outward faces only (seen from the bus's own centre)
BODIES['cityBus'].update(tierFritMerge=2.0, tierFritMax=200, nearGlassSkip=('Window_',), nearReseatPlates=True,
                         farTyreSegments=18,
                         farFlatPrefixes=('SkirtPanel_', 'DoorLeaf_', 'AccessDoor_', 'Panel_FR', 'Panel_RR'),
                         planarOuterOnly=(0.0, 0.0, 1.6))
# cityBus: the raked-edge black bands on the last side windows were laid along the cage opening polygon, not the
# rounded pane outline, and fell back to x = 1.29 m where the ray missed the shell: a dark strip standing 25 mm proud of
# the flank and offset from the glass, read in game as a flap sticking out of the body. The pane's own frit already
# borders that edge, so the band is not exported (`exportDrop`)
BODIES['cityBus']['exportDrop'] = ('Glass_SideL5_SlopeBand', 'Glass_SideR4_SlopeBand')
# patrolSedan LOD2: the near build's thin tail-lamp housing walls at the lid shut lines, widened by the far collapse,
# were dark notches inside the red tail band; those black body faces become tail lamp (no vertex moves)
BODIES['patrolSedan']['farLampZones'] = (('VehicleTaillamp', ('VehiclePaint',), (0.52, 0.62), (2.2, 2.8), (0.865, 0.995)),)
# sedan family: the retrofit build is 3k triangles heavier and the near detail pass halved the steering rim (576 ->
# 398 vertices), which moved its fitted centre 1.4 mm off the SteeringWheel pivot and broke the driver's hand contacts
# (verify:vehicle-contact-pose); `nearKeepWhole` keeps it out of that pass, the other detail parts take the share
for _b in ('sedan', 'sedanSport', 'sedanLuxe'):
    BODIES[_b]['nearKeepWhole'] = ('SteeringWheel_Rim',)
# every other body whose LOD0 rim read back under its source count (source 576 vertices, the bus 192): the same pass
# had cut it to 42-564 (cityBus 42, pickup 144, patrolSedan 186, boxTruck 212, minivan 236, compactHatchCross 261,
# wagon 316, deliveryVan 338, taxiCheckered 370, taxiAdBoard 376, taxi 397, compactHatchSport 403, compactHatch 430,
# coupe 564); the rim is kept whole and the other detail parts take the share
for _b in ('cityBus', 'pickup', 'patrolSedan', 'boxTruck', 'minivan', 'compactHatch', 'compactHatchSport',
           'compactHatchCross', 'wagon', 'deliveryVan', 'taxi', 'taxiCheckered', 'taxiAdBoard', 'coupe'):
    BODIES[_b]['nearKeepWhole'] = ('SteeringWheel_Rim',)
# articulation draw budget (8 shut / 32 pool draws per part set, verify:vehicle-articulation), LOD0 and the wagon's
# LOD1 only, applied as each GLB is written (`exportFold`), so no vertex and no other tier changes. On the doors and
# lids the tinted rear glass draws as the near tier's rebuilt VehicleGlass, as every one of these bodies shipped it
# before the retrofit; the door-glass frit ribbon draws as the pane's glass (one window draw per door less); the wagon
# tailgate's void black joins trim black, and at its mid tier the door and lid chrome is trim black (as the minivan's).
_TINT = ((0,), 'parts', {'VehicleGlassTint': 'VehicleGlass'})
_FRIT = ((0,), 'windows', {'VehicleTrimBlack': 'VehicleGlass'})
for _b, _rules in (('compactHatch', (_TINT, _FRIT)), ('compactHatchSport', (_TINT, _FRIT)), ('compactHatchCross', (_TINT, _FRIT)),
                   ('fullSizeSuv', (_TINT, _FRIT)), ('compactSuv', (_TINT, _FRIT)), ('minivan', (_TINT,)), ('patrolSuv', (_TINT,)),
                   ('taxiCheckered', (_FRIT,)), ('taxiAdBoard', (_FRIT,)),
                   ('wagon', (((0,), 'parts', {'VehicleGlassTint': 'VehicleGlass', 'VehicleVoid': 'VehicleTrimBlack'}),
                              ((1,), 'parts', {'VehicleChrome': 'VehicleTrimBlack'})))):
    BODIES[_b]['exportFold'] = _rules

# ------------------------------------------------------------------------------------------- plate cards
# 2026-10-05: every skill body's runtime plate card fits the plate pocket its build gives it (`plateCard`, written
# as the anchor's plateW / plateH extras and rotation). The pocket is the opening of the plate recess, mount or
# paint zone round the anchor, or where the build has none the plate blank it carried there (removed at export):
# measured on the LOD0 GLBs (rays along the body axis through the anchor, x = 0 / +-0.1 m) and read from parts.json
# and the blend. A pocket of 156 mm or more takes a real US plate, 305 x 152 mm; a shorter one takes that plate
# scaled uniformly to the pocket height less 5 mm. `tilt: 'surface'` where the mount is raked (3 deg or more).
# An anchor with no pocket and no blank keeps the runtime's default card (no key). verify:vehicle-plate-card holds
# every card inside its pocket on all three tiers.
def _plate_card(pocket_mm, tilt=False):
    """The card for a pocket `pocket_mm` tall: a US 305 x 152 plate, or that plate scaled to the pocket less 5 mm."""
    h = 0.152 if pocket_mm >= 156 else (pocket_mm - 5) / 1000
    card = dict(w=round(0.305 * h / 0.152, 3), h=round(h, 3))
    if tilt:
        card['tilt'] = 'surface'
    return card


PLATE_CARDS = {
    # sedan family: front black plate zone z 0.400-0.515 (115 mm; its station moved to the zone centre), rear recess
    # z 0.580-0.695 (115 mm)
    ('sedan', 'sedanSport', 'sedanLuxe'): dict(front=_plate_card(115), rear=_plate_card(115)),
    # compact SUV and its patrol trim: front recess z 0.540-0.655 (115 mm) on a 5 deg raked housing, rear recess
    # z 0.748-0.862 (114 mm)
    ('compactSuv', 'patrolSuv'): dict(front=_plate_card(115, tilt=True), rear=_plate_card(114)),
    # hatch family: front blank 520 x 112 mm (no recess; the card stands on the bumper over the intake as the blank
    # did), rear recess z 0.530-0.650 (120 mm)
    ('compactHatch', 'compactHatchSport', 'compactHatchCross'): dict(front=_plate_card(112), rear=_plate_card(120)),
    # wagon: front black plate zone z 0.430-0.542 (112 mm), rear tailgate recess z 0.618-0.744 (126 mm, 4.6 deg rake)
    ('wagon',): dict(front=_plate_card(112), rear=_plate_card(126, tilt=True)),
    # minivan: front recess z 0.436-0.568 (132 mm), rear recess z 0.672-0.852 (180 mm)
    ('minivan',): dict(front=_plate_card(132), rear=_plate_card(180)),
    # taxi family: front flat mount 330 x 170 mm, rear lid recess z 0.735-0.895 (160 mm) on the 21.6 deg lid rake
    ('taxi', 'taxiCheckered', 'taxiAdBoard'): dict(front=_plate_card(170), rear=_plate_card(160, tilt=True)),
    # full-size SUV: front, the build's 520 x 112 mm plate mount (removed, `plateBlanks`); rear tailgate recess
    # z 1.035-1.200 (165 mm, 6.3 deg rake)
    ('fullSizeSuv',): dict(front=_plate_card(112), rear=_plate_card(165, tilt=True)),
    # delivery van: front recess z 0.586-0.698 (112 mm); the rear has no pocket (flat bumper moulding; the build's
    # blank sat off centre, x 0-0.51) and keeps the default card
    ('deliveryVan',): dict(front=_plate_card(112)),
    # pickup: rear step-bumper pocket z 0.586-0.751 (165 mm); the front has no pocket or blank (default card)
    ('pickup',): dict(rear=_plate_card(165)),
    # box truck: front recess inside its surround z 0.575-0.735 (160 mm; its station moved to the recess centre);
    # the rear plate rides on a bracket 80 mm proud of the step bumper in the build, which the anchor (3-9 mm proud
    # contract) cannot reach: default card
    ('boxTruck',): dict(front=_plate_card(160)),
    # city bus: rear backing panel PlateRear_Recess z 0.460-0.640 (180 mm); the front recess lies inside the bumper
    # skin, so the front has no visible pocket (default card)
    ('cityBus',): dict(rear=_plate_card(180)),
    # patrol sedan: front, the build's own 305 x 152 mm accessory blank; rear recess z 0.445-0.640 (195 mm; its
    # station moved down to the build's blank, z 0.454-0.606)
    ('patrolSedan',): dict(front=dict(w=0.305, h=0.152), rear=_plate_card(195)),
    # exotic coupe: rear diffuser-face slot z 0.451-0.544 (94 mm, steep walls); the front has no pocket (default card)
    ('exoticCoupe',): dict(rear=_plate_card(94)),
}
for _bodies, _cards in PLATE_CARDS.items():
    for _b in _bodies:
        BODIES[_b]['plateCard'] = _cards

CFG = None   # the selected BODIES entry, set in main()
# LOD0 is held under the near share, not only the 165k tier cap: the overlay reserves
# nearMax x the dearest near BAKE inside its 800k triangle share (budget.worstCase), and the
# dearest mid (26,170) and far (6,124) bakes already on the fleet leave 146,670 for a near bake.
CAPS = [147000, 22000, 6000]
FRAME_KEY, FRAME = 'vehicleFrame', 'lhd-v1'

# Skill contract material -> engine material. Two sources may share a target (the tinted rear
# glass joins the one glass graph the overlay knows how to draw); they are merged, not suffixed.
MATERIALS = {
    'Paint': 'VehiclePaint', 'Paint_Accent': 'VehicleTrimGloss',
    'Glass': 'VehicleGlass', 'GlassTint': 'VehicleGlass', 'Lens_Tail': 'VehicleTaillampLens',
    'Trim_Black': 'VehicleTrimBlack', 'Trim_Satin': 'VehicleTrimSatin', 'Trim_Chrome': 'VehicleChrome',
    'Metal_Brushed': 'VehicleSteel', 'Metal_Cast': 'VehicleMetalCast', 'Metal_Dark': 'VehicleMetalDark',
    'Tyre': 'VehicleTyre', 'Rim': 'VehicleAlloy', 'Brake': 'VehicleBrakeDisc', 'Caliper': 'VehicleCaliper',
    'Lamp_Head': 'VehicleHeadlamp', 'Lamp_Tail': 'VehicleTaillamp', 'Lamp_Indicator': 'VehicleIndicator',
    'Reflector': 'VehicleLampReflector', 'Lamp_DRL': 'VehicleDRL', 'Lamp_Reverse': 'VehicleReverse',
    'Interior': 'VehicleInterior', 'Interior_Accent': 'VehicleSeatInsert',
    'Interior_Trim': 'VehicleInteriorTrim', 'Screen': 'VehicleScreen',
    'Underbody': 'VehicleUnderbody', 'Void': 'VehicleVoid', 'Mirror': 'VehicleMirror',
    'Plate': 'VehiclePlate',
    # compactSuv palette: two-tone alloy, the ceramic frit round the glass (opaque black, never
    # glass) and the clear headlamp cover (no lamp channel of its own)
    'Rim_Dark': 'VehicleAlloyDark', 'Frit': 'VehicleTrimBlack', 'Lens_Head': 'VehicleHeadlampLens',
    'Sign': 'VehicleSign',   # roof-sign face (the name the shipped taxi uses)
    # the pickup's smoked reverse-lamp cover (the name the pickup build's own export uses)
    'Lens_Smoke': 'VehicleReverseLens',
    # dry-van body materials
    'Box_Alu': 'VehicleBoxAluminium', 'Box_Roof': 'VehicleBoxRoof', 'Box_Panel': 'VehicleBoxPanel', 'Cargo_Liner': 'VehicleCargoLiner',
    'Lens_Amber': 'VehicleIndicatorLens',
    # livery, sign and emergency-lens materials: the untinted names the shipped trim bodies use
    'Livery_Black': 'VehicleLiveryBodyBlack', 'Livery_White': 'VehicleLiveryWhite', 'Livery_Dark': 'VehicleLiveryDark',
    'Livery_Yellow': 'VehicleLiveryYellow', 'Lamp_EmergencyRed': 'VehicleEmergencyRed',
    'Lamp_EmergencyBlue': 'VehicleEmergencyBlue',
    # the light bar's centre takedown module: a frosted white cover over a reflector lamp body, never lit and never
    # one of the emergency channels (a clear 0.07 lens or the cabin glass there read as an empty gap in the bar)
    'Lens_Frost': 'VehicleLampLensFrost',
}
SIGN_NODES = ('TaxiSignBase', 'TaxiSign', 'Antenna')   # kept whole on every tier (the trims swap the sign)
GLASS = ('VehicleGlass', 'VehicleGlassTint')   # every glass material (`tintAsGlass` bodies, the bus frit merge)

# mid/far glass (`tierGlass` bodies). LOD1 and LOD2 carry no cabin (the far tier none at all, the mid tier only a dark
# lining), so the near tier's light, mostly clear glass reads as pale panes at range. On those tiers only, the glass
# leaves darker and more opaque, the tinted rear glass darker still. `tierGlass` may be a dict overriding either entry.
TIER_GLASS = {
    'VehicleGlass': dict(baseColor=(0.06, 0.064, 0.068, 1.0), alpha=0.75),
    'VehicleGlassTint': dict(baseColor=(0.025, 0.026, 0.028, 1.0), alpha=0.85),
}


def glass_names():
    """The materials the cage tiers treat as glass (decal skip, the greenhouse lining, the seam outline test):
    VehicleGlass, and VehicleGlassTint too on a `tintAsGlass` body (one whose dark rear glass keeps its own
    VehicleGlassTint and should be handled as glass, not as trim)."""
    return GLASS if CFG.get('tintAsGlass') else ('VehicleGlass',)


def material_names():
    """MATERIALS with the selected body's overrides (same key order, so merges happen in the same order)."""
    return {**MATERIALS, **(CFG.get('materials') or {})}

SHELL = ('Body', 'HoodSkin', 'TrunkSkin', 'Panel_FL', 'Panel_FR', 'Panel_RL', 'Panel_RR', 'Panel_SlideR')   # + the sliding door skin


def log(**kw):
    print(json.dumps(kw), flush=True)


def meshes():
    return [o for o in bpy.context.scene.objects if o.type == 'MESH' and '_shadingRef' not in o]


def triangles(ob):
    ob.data.calc_loop_triangles()
    return len(ob.data.loop_triangles)


def total_triangles():
    return sum(triangles(o) for o in meshes())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


# --------------------------------------------------------------------------- preparation

def drop_stage_and_keys():
    # `exportDrop`: build parts that never ship at any tier (dropped with the stage parts, before every reduction)
    removed = [o.name for o in list(bpy.data.objects)
               if o.get('zstack') == 'stage' or o.name in CFG.get('exportDrop', ())]
    for name in removed:
        bpy.data.objects.remove(bpy.data.objects[name], do_unlink=True)
    keyed = 0
    for ob in meshes():
        if ob.data.shape_keys:
            keyed += len(ob.data.shape_keys.key_blocks)
            ob.shape_key_clear()
    return removed, keyed


def rename_materials():
    targets = {}
    for src, dst in material_names().items():
        mat = bpy.data.materials.get(src)
        if mat is None:
            continue
        if dst in targets:
            keep = targets[dst]
            for ob in meshes():
                for slot in ob.material_slots:
                    if slot.material is mat:
                        slot.material = keep
            bpy.data.materials.remove(mat)
            continue
        mat.name = dst
        if mat.name != dst:
            raise ValueError(f'material {src} could not take the engine name {dst}')
        targets[dst] = mat
    unknown = sorted({s.material.name for o in meshes() for s in o.material_slots
                      if s.material and not s.material.name.startswith('Vehicle')})
    if unknown:
        raise ValueError(f'materials without an engine name: {unknown}')
    for mat in bpy.data.materials:
        # Double-sided, as every other fleet body exports them. The overlay keeps ONE prepared
        # material per name for the whole fleet, built from whichever body's GLB bakes first, so
        # a single-sided VehicleTrimBlack or VehicleInterior here would make every other car's
        # cabin lining cull from behind whenever this body happened to load first (measured:
        # boxTruck, deliveryVan and pickup mid greenhouses leaked 10-32 % in that order).
        mat.use_backface_culling = False
    return sorted(targets)


def flatten_transmission():
    """Export transmissive materials as alpha-blended glass, never KHR_materials_transmission.

    three renders a transmissive material with a ViewportTextureNode that copies the framebuffer
    mid-pass; built during the environment probe bake it ends a render pass the bake ends again,
    and the boot GPU gate rejects the boot ("RenderPassEncoder was already ended"). The overlay
    rebuilds VehicleGlass itself, but the lamp lens covers reach the renderer as exported. Zero the
    transmission and carry the see-through as alpha (alpha = 1 - 0.75 * transmission), both faces
    kept (the same rule as scripts/assets/export-urban-utility-lods.py).
    """
    flattened = []
    for material in bpy.data.materials:
        if not material.use_nodes or not material.node_tree:
            continue
        for node in material.node_tree.nodes:
            if node.type != 'BSDF_PRINCIPLED':
                continue
            weight = node.inputs.get('Transmission Weight')
            if weight is None or weight.is_linked or weight.default_value <= 0.0:
                continue
            alpha = node.inputs.get('Alpha')
            if alpha is not None and not alpha.is_linked:
                alpha.default_value = min(alpha.default_value, 1.0 - 0.75 * weight.default_value)
                material.surface_render_method = 'BLENDED'
            weight.default_value = 0.0
            material.use_backface_culling = False
            flattened.append(material.name)
    return sorted(set(flattened))


def assert_runtime_glb(path):
    """Fail the export if a GLB would still break the boot: morph targets, transmission, raw names."""
    raw = Path(path).read_bytes()
    doc = json.loads(raw[20:20 + int.from_bytes(raw[12:16], 'little')])
    morphs = sum(len(p.get('targets', [])) for m in doc.get('meshes', []) for p in m['primitives'])
    names = {m.get('name') for m in doc.get('materials', [])}
    raw_names = sorted(n for n in names if not (n or '').startswith('Vehicle'))
    extensions = doc.get('extensionsUsed', [])
    if morphs or 'KHR_materials_transmission' in extensions or 'KHR_materials_volume' in extensions or raw_names:
        raise ValueError(f'{Path(path).name} is not runtime-safe: {morphs} morph targets, '
                         f'extensions {extensions}, raw material names {raw_names}')
    return dict(morphTargets=0, extensions=extensions)


RENAMES = {
    'Hood_Skin': 'HoodSkin', 'Trunk_Skin': 'TrunkSkin',
    'Glass_Windscreen': 'Windscreen', 'Glass_Rear': 'RearWindow',
    'Glass_Back': 'RearWindow', 'Glass_Back_Seal': 'RearWindow_Seal',   # liftgate glass name
    'Glass_Windscreen_Seal': 'Windscreen_Seal', 'Glass_Rear_Seal': 'RearWindow_Seal',
    'Cabin_Floor': 'FloorPan',
    'SeatMesh_FL_Back': 'Seat_FL_Back', 'SeatMesh_FR_Back': 'Seat_FR_Back',
    'SteeringWheelMesh_Rim': 'SteeringWheel_Rim', 'SteeringWheelMesh_Hub': 'SteeringWheel_Hub',
    'SteeringWheelMesh_Column': 'SteeringColumn',
    # a top-hinged tailgate is the runtime's `Trunk` lid
    'Tailgate': 'Trunk', 'Tailgate_Skin': 'TrunkSkin',
    # [exoticCoupe run] the mid-engine car's front-hinged engine cover is the runtime's rear lid (`Trunk`); only on a
    # body with `engineCoverTrunk` (the bus's rear engine lid is not a runtime lid and keeps its own name)
    'EngineCover': 'Trunk', 'EngineCover_Skin': 'TrunkSkin',
    # the cushions are the seat bases the runtime seats occupants and entry anchors on
    'SeatMesh_FL_Cushion': 'Seat_FL_Base', 'SeatMesh_FR_Cushion': 'Seat_FR_Base',
    'SeatMesh_Rear_Cushion': 'Seat_Rear_Base',
}


ENGINE_COVER_NODES = ('EngineCover', 'EngineCover_Skin')


def rename_nodes():
    done = {}

    def rename(ob, new):
        if bpy.data.objects.get(new) not in (None, ob):
            raise ValueError(f'cannot rename {ob.name} to {new}: name taken')
        done[ob.name] = new
        ob.name = new
        if ob.data is not None and ob.data.users == 1:
            ob.data.name = new

    for old, new in list(RENAMES.items()) + list((CFG.get('renames') or {}).items()):   # + per-body renames
        if old in ENGINE_COVER_NODES and not CFG.get('engineCoverTrunk'):
            continue
        ob = bpy.data.objects.get(old)
        if ob:
            rename(ob, new)
    for c in (CFG.get('slideDoors') or ()):          # sliding doors
        ob = bpy.data.objects.get(f'SlideDoor_{c}_Skin')
        if ob:
            rename(ob, f'Panel_Slide{c}')
    for c in ('FL', 'FR', 'RL', 'RR'):
        for old, role in ((f'Door_{c}_Skin', None), (f'Tyre_{c}', 'Tyre'), (f'Rim_{c}', 'Lip'),
                          (f'Disc_{c}', 'Disc'), (f'Caliper_{c}', 'Caliper')):
            ob = bpy.data.objects.get(old)
            if ob:
                rename(ob, f'Panel_{c}' if role is None else f'Wheel_{c}_{role}')
    for c in ('RL', 'RR'):          # dual rear pairs: outer tyre / rim are the runtime's, the inner ride along
        for old, new in ((f'Tyre_{c}O', f'Wheel_{c}_Tyre'), (f'Tyre_{c}I', f'Wheel_{c}_TyreInner'),
                         (f'Rim_{c}O', f'Wheel_{c}_Lip'), (f'Rim_{c}I', f'Wheel_{c}_LipInner'),
                         (f'Tyre_{c}O_Side', f'Wheel_{c}_TyreSide'), (f'Tyre_{c}I_Side', f'Wheel_{c}_TyreInnerSide')):
            ob = bpy.data.objects.get(old)
            if ob:
                rename(ob, new)
    spokes = sorted((o for o in bpy.data.objects if o.name.startswith('SteeringWheelMesh_Spoke')), key=lambda o: o.name)
    for i, ob in enumerate(spokes):
        rename(ob, f'SteeringWheel_Spoke{i}')
    # rear headrests carry their origin at the bench; the runtime places each rear occupant at
    # its headrest node, so move each origin onto its own geometry (vertices stay put) and name
    # the two outer ones in the runtime's order
    rests = [o for o in bpy.data.objects if o.name.startswith('SeatMesh_Rear_Headrest') and o.type == 'MESH']
    for ob in rests:
        centre = sum((Vector(c) for c in ob.bound_box), Vector()) / 8
        ob.data.transform(Matrix.Translation(-centre))
        ob.matrix_world = ob.matrix_world @ Matrix.Translation(centre)
    rests.sort(key=lambda o: -o.matrix_world.translation.x)
    if len(rests) >= 2:
        rename(rests[0], 'Seat_Rear_Headrest0')
        rename(rests[-1], 'Seat_Rear_Headrest1')
        for i, ob in enumerate(rests[1:-1]):
            rename(ob, f'Seat_Rear_Headrest{2 + i}')
    # a pedal box of three, left to right in the driver's view (+X is the car's left):
    # clutch, brake, accelerator
    pedals = sorted((o for o in bpy.data.objects if o.name.startswith('Pedal') and o.type == 'MESH'),
                    key=lambda o: -(o.matrix_world @ Vector(o.bound_box[0]).lerp(Vector(o.bound_box[6]), 0.5)).x)
    if len(pedals) == 3:
        for ob, new in zip(pedals, ('PedalClutch', 'PedalBrake', 'PedalThrottle')):
            rename(ob, new)
    # a body without a bonnet / boot lid (noLids) and with windowless doors (windowDoors lists the glazed ones)
    lids = () if CFG.get('noLids') else ('HoodSkin',)
    doors_ = CFG.get('doors', ('FL', 'FR', 'RL', 'RR'))
    missing = [n for n in ('Body',) + lids + ('Windscreen', 'FloorPan', 'Dash',
                           'SteeringWheel_Rim', 'Seat_FL', 'Seat_FR', 'PedalBrake', 'PedalThrottle')
               + (() if CFG.get('noTrunk') or CFG.get('noLids') else ('TrunkSkin',))
               + (() if CFG.get('noRearWindow') else ('RearWindow',))
               + tuple(f'{k}_{c}' for k in ('Door', 'Panel') for c in doors_)
               + tuple(f'Window_{c}' for c in CFG.get('windowDoors', doors_))
               + tuple(f'{k}{c}' for k in ('SlideDoor_', 'Panel_Slide') for c in (CFG.get('slideDoors') or ()))
               + tuple(f'Wheel_{c}' for c in ('FL', 'FR', 'RL', 'RR'))
               if not bpy.data.objects.get(n) and n not in CFG.get('optionalNodes', ())]
    if missing:
        raise ValueError(f'source lacks runtime nodes: {missing}')
    return done


ARTICULATED_MATERIALS = {
    'VehicleInterior': 'VehicleTrimBlack', 'VehicleTrimGloss': 'VehicleTrimBlack',
    'VehicleLampReflector': 'VehicleTrimBlack', 'VehicleSteel': 'VehicleChrome',
    'VehicleMirror': 'VehicleChrome',
}


def detach_lid_static_parts():
    """`lidStaticParts` (node-name prefixes): small lid-mounted parts that ride on the body instead.

    A part moved off its lid leaves that lid's draw pools. It keeps its world placement and the material the lid
    fold gave it, so the shut car renders unchanged. When the lid opens, the part stays with the body. The runtime
    opens no lid today: VehicleArticulation.setLid has no caller.
    """
    out = []
    for ob in [o for o in bpy.context.scene.objects if o.name.startswith(tuple(CFG.get('lidStaticParts') or ()))]:
        mw = ob.matrix_world.copy()
        ob.parent = None
        ob.matrix_world = mw
        for slot in ob.material_slots:
            name = slot.material.name if slot.material else ''
            dst = ARTICULATED_MATERIALS.get(name) or (CFG.get('articulatedFold') or {}).get(name)
            if dst:
                slot.link = 'OBJECT' if ob.data.users > 1 else slot.link
                slot.material = bpy.data.materials[dst]
        out.append(ob.name)
    if out:
        log(stage='lidStaticParts', detached=out)
    return out


def consolidate_articulated_materials(tier='near'):
    """Fold the minor materials on doors and lids into the fleet's trim set.

    Every moving panel is baked twice, shut and per part, one draw per material each
    (verify-vehicle-articulation holds the shut batch to 8 draws and the part pools to 32). The
    skill build spreads its doors over 13 materials; on the panels only, the door card lining,
    the lamp cups and a 13-triangle accent sliver become trim black, and the door pull and mirror
    glass become chrome - the fleet's own choices for those parts.
    """
    roots = [bpy.data.objects[n] for n in ('Door_FL', 'Door_FR', 'Door_RL', 'Door_RR', 'Hood', 'Trunk', 'SlideDoor_R', 'SlideDoor_L')
             if bpy.data.objects.get(n)]
    moved = {}
    for root in roots:
        for ob in [root] + list(root.children_recursive):
            if ob.type != 'MESH':
                continue
            for slot in ob.material_slots:
                name = slot.material.name if slot.material else ''
                # per-body folds (`articulatedFold` on every tier, `articulatedFoldMid` at the mid tier)
                # keep a body's door and lid pools inside the articulation draw budget
                dst = (ARTICULATED_MATERIALS.get(name) or (CFG.get('articulatedFold') or {}).get(name)
                       or ((CFG.get('articulatedFoldMid') or {}).get(name) if tier == 'mid' else None))
                # a recess cup riding on a lid (the tailgate's plate housing) is trim black too:
                # one satin batch fewer on the part pools
                if (dst is None and ob.name.endswith('_Housing') and slot.material
                        and slot.material.name == 'VehicleTrimSatin'):
                    dst = 'VehicleTrimBlack'
                # `articulatedFoldFinal`: a last fold applied to the result (or to the name when nothing
                # folded it), so trim black gathered from the minor materials can join a livery on the panels
                final = (CFG.get('articulatedFoldFinal') or {}).get(dst or name)
                if final:
                    dst = final
                if dst:
                    moved[slot.material.name] = moved.get(slot.material.name, 0) + 1
                    slot.link = 'OBJECT' if ob.data.users > 1 else slot.link
                    slot.material = bpy.data.materials[dst]
    shut = sorted({s.material.name for r in roots for ob in [r] + list(r.children_recursive)
                   if ob.type == 'MESH' for s in ob.material_slots if s.material})
    return dict(remapped=moved, panelMaterials=shut)


def rewrite_hinges():
    """Door axes into the glTF node frame; lids into the runtime's `lidHinge` form.

    The skill writes `hingeAxis` in Blender axes. The runtime reads a door's vector axis in the
    door node's own glTF frame, where Blender (x, y, z) is (x, z, -y); a door node here carries no
    rest rotation, so the rewritten vector is exactly the axis the door swings about.
    """
    out = {}
    for ob in bpy.data.objects:
        if 'hingeAxis' not in ob:
            continue
        x, y, z = (float(v) for v in ob['hingeAxis'])
        limit = float(ob.get('openLimit', 1.0))
        sign = -1 if float(ob.get('openSign', 1)) < 0 else 1
        rest = ob.matrix_world.to_3x3()
        if max(abs(rest[i][j] - (1.0 if i == j else 0.0)) for i in range(3) for j in range(3)) > 1e-5:
            raise ValueError(f'{ob.name}: hinge pivot carries a rest rotation')
        if ob.name.startswith('SlideDoor_'):
            # a sliding door translates; the runtime bakes it static today and reads these extras later
            path = [list(p) for p in ob['slidePath']] if 'slidePath' in ob else [[0, 0, 0]] * 4
            for key in ('hingeAxis', 'hingeAxisGltf', 'openLimit', 'openSign', 'slideKnots', 'slidePath', 'openDir'):
                if key in ob:
                    del ob[key]
            ob['doorType'] = 'sliding'
            ob['slideOutM'] = round(abs(path[2][0]), 4)
            ob['slideTravelM'] = round(path[-1][1], 4)
            ob['slideAxisGltf'] = [0.0, 0.0, -1.0]
            ob['windowDropM'] = 0.0
            out[ob.name] = ['slide', ob['slideOutM'], ob['slideTravelM']]
            continue
        if ob.name.startswith('Door_'):
            ob['hingeAxis'] = [x, z, -y]
            # barn doors on a noTrunk body (the panel van): no pane to drop, and the skill's
            # `openDir` is not a runtime key. Keyed on the body, so other barn-door bodies keep the
            # extras they were reviewed with (neither key changes what the runtime draws).
            if CFG.get('noTrunk'):
                if ob.get('doorType') == 'barn':
                    ob['windowDropM'] = 0.0
                if 'openDir' in ob:
                    del ob['openDir']
        elif ob.name in ('Hood', 'Trunk'):
            if abs(x) < 0.999:
                raise ValueError(f'{ob.name}: lid hinge is not the body X axis')
            ob['hingeAxis'] = 'X'
            sign = sign if x > 0 else -sign
        else:
            continue
        ob['openSign'] = sign
        ob['openLimitRadians'] = limit
        for key in ('hingeAxisGltf', 'openLimit'):
            if key in ob:
                del ob[key]
        out[ob.name] = [ob['hingeAxis'] if isinstance(ob['hingeAxis'], str) else list(ob['hingeAxis']), sign, limit]
    for ob in bpy.data.objects:
        if 'spinAxisGltf' in ob:
            del ob['spinAxisGltf']
    return out


def add_plates():
    """PlateFront / PlateRear anchors: the runtime prints its own registration card there.

    The rear sits in the body's plate recess, the front on the bumper face between the grille
    and the lower intake; each is found by casting onto the exported shell, never by guessing.
    """
    dg = bpy.context.evaluated_depsgraph_get()
    # the shell and the recess housings that close its cut-outs (the rear plate sits in one)
    # the skill render's plate blanks give way to the runtime's card; the flat mount stays as its seat
    for o in [o for o in meshes() if o.name.startswith('PlateBlank') or o.name in CFG.get('plateBlanks', ())]:
        bpy.data.objects.remove(o, do_unlink=True)
    targets = [o for o in meshes() if o.name == 'Body' or o.name.endswith('_Housing') or o.name.startswith(('Intake', 'PlateMount'))
               or o.name in CFG.get('plateTargets', ())]
    trees = [(o, BVHTree.FromObject(o, dg)) for o in targets]
    mat = bpy.data.materials.get('VehiclePlate') or bpy.data.materials.new('VehiclePlate')
    mat.diffuse_color = (0.9, 0.9, 0.88, 1.0)
    out = {}
    for name, z, face in CFG['plates']:
        best = None
        for ob, tree in trees:
            inv = ob.matrix_world.inverted()
            origin = inv @ Vector((0.0, face * (3.5 if CFG['lengthM'] < 6.0 else CFG['lengthM'] / 2 + 1.0), z))   # a 7.6 m body
            direction = (inv.to_3x3() @ Vector((0.0, -face, 0.0))).normalized()
            hit, normal = tree.ray_cast(origin, direction, 6.0)[:2]
            if hit is not None:
                world = ob.matrix_world @ hit
                if best is None or face * world.y > face * best[1].y:
                    best = (ob.name, world, (ob.matrix_world.inverted().transposed().to_3x3() @ normal).normalized())
        if best is None:
            raise ValueError(f'{name}: no shell under the plate anchor')
        p = best[1] + Vector((0.0, face * CFG.get('plateProudM', 0.003), 0.0))
        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1.0)
        bmesh.ops.scale(bm, vec=Vector((0.305, 0.004, 0.152)), verts=bm.verts)
        me = bpy.data.meshes.new(name)
        bm.to_mesh(me)
        bm.free()
        me.materials.append(mat)
        ob = bpy.data.objects.new(name, me)
        bpy.context.scene.collection.objects.link(ob)
        ob.location = p
        out[name] = [round(v, 4) for v in p]
        card = (CFG.get('plateCard') or {}).get('front' if face < 0 else 'rear')
        if card:
            # `plateCard`: the runtime draws this anchor's card at its own size (node extras plateW / plateH, metres)
            # and at the node's rotation; `tilt: 'surface'` turns the anchor about X onto the surface the ray hit
            # (a raked mount). Anchors without the key keep the fleet's default card, unrotated. The turn is held
            # in `_plateRotX` and set only as each tier is written (`export`): the cage tiers' decal and lining
            # passes see the anchor's plate box as they always did, so no other mesh changes with the card.
            ob['plateW'] = float(card['w'])
            ob['plateH'] = float(card['h'])
            rot = 0.0
            if card.get('tilt') == 'surface':
                n = best[2]
                rot = math.atan2(face * n.z, face * n.y)   # (0, face, 0) about X onto n
                ob['_plateRotX'] = rot
            out[name] = dict(at=out[name], w=card['w'], h=card['h'], rotXDeg=round(math.degrees(rot), 2))
    return out


SHADING_REFS = {}   # shell object name -> its dense outer surface, kept by planar_shell


def restore_shading():
    """Give each merged shell back the dense surface's own normals, vertex for vertex.

    The planar merge only removes vertices; every vertex it keeps sits exactly where it was. Its
    long sliver triangles, though, re-average each kept vertex's normal over a few big faces, so a
    gently curved door shades as flat triangular patches. For every corner of the merged mesh the
    dense mesh's corner normal at the SAME vertex position is written back as a custom split normal
    (of that vertex's dense corners, the one facing most like the merged face, so a sharp panel edge
    stays sharp). Runs last in the near preparation; corners with no dense match keep their normal.
    """
    from mathutils.kdtree import KDTree
    done = {}
    for name, ref in list(SHADING_REFS.items()):
        ob = bpy.data.objects.get(name)
        if ob is None:
            continue
        rme = ref.data
        kd = KDTree(len(rme.vertices))
        for v in rme.vertices:
            kd.insert(v.co, v.index)
        kd.balance()
        by_vert = {}
        rn = rme.corner_normals
        for poly in rme.polygons:
            for li in poly.loop_indices:
                by_vert.setdefault(rme.loops[li].vertex_index, []).append(Vector(rn[li].vector))
        me = ob.data
        cur = me.corner_normals
        normals = [Vector(cur[i].vector) for i in range(len(me.loops))]
        matched = 0
        for poly in me.polygons:
            fn = poly.normal
            for li in poly.loop_indices:
                co = me.vertices[me.loops[li].vertex_index].co
                _, idx, dist = kd.find(co)
                if idx is None or dist > 1e-5 or idx not in by_vert:
                    continue
                normals[li] = max(by_vert[idx], key=lambda n: n.dot(fn))
                matched += 1
        me.normals_split_custom_set(normals)
        data = ref.data
        bpy.data.objects.remove(ref, do_unlink=True)
        bpy.data.meshes.remove(data)
        done[name] = matched
    SHADING_REFS.clear()
    return done


def planar_shell(angle_deg):
    """Merge nearly coplanar faces of the shell and panel skins, then re-triangulate.

    Geometric-error bounded, not a face budget: two faces merge only when their normals differ by
    less than `angle_deg`, material zones, seams, sharp edges and every open boundary (the panel
    gaps) are delimiters, so the silhouette, the paint/cladding split and each shut line stay put.
    A 0.5 degree limit moves no surface point by more than about 2 mm over a 0.5 m panel.
    """
    out = []
    for name in SHELL:
        ob = bpy.data.objects.get(name)
        if ob is None:
            continue
        before = triangles(ob)
        # the dense surface's own shading, kept to restore the normals after the merge
        # (outer surface only: the 4 mm inner wall lies closer than the curvature to many outer
        # faces, and its inward normals must never be picked up)
        ref = ob.copy()
        ref.data = ob.data.copy()
        bpy.context.scene.collection.objects.link(ref)
        slots = [sl.material.name if sl.material else '' for sl in ob.material_slots]
        inner = {i for i, n in enumerate(slots) if n == 'VehicleInterior'}
        rbm = bmesh.new()
        rbm.from_mesh(ref.data)
        bmesh.ops.delete(rbm, geom=[f for f in rbm.faces if f.material_index in inner], context='FACES')
        rbm.to_mesh(ref.data)
        rbm.free()
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        dv, de = list(bm.verts), list(bm.edges)
        if CFG.get('planarOuterOnly'):
            # `planarOuterOnly`: only outward-facing faces merge (seen from a centre point, (0, 0, 0.9) or the
            # key's own). With the 0.2 mm edge roll, an inward-facing paint ring behind the nose merged into one
            # ngon whose outline skipped the grille openings, and its triangulation drew a paint sheet across them
            ctr = Vector(CFG['planarOuterOnly']) if isinstance(CFG['planarOuterOnly'], (list, tuple)) else Vector((0.0, 0.0, 0.9))
            out_ = lambda f_: f_.normal.dot(f_.calc_center_median() - ctr) > 0.0
            dv = [v for v in bm.verts if all(out_(f_) for f_ in v.link_faces)]
            de = [e for e in bm.edges if all(out_(f_) for f_ in e.link_faces)]
        bmesh.ops.dissolve_limit(bm, angle_limit=math.radians(angle_deg), use_dissolve_boundaries=False,
                                 verts=dv, edges=de, delimit={'MATERIAL', 'SEAM', 'SHARP'})
        bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if len(f.verts) > 3],
                              quad_method='BEAUTY', ngon_method='BEAUTY')
        bm.to_mesh(ob.data)
        bm.free()
        ob.data.update()
        ref['_shadingRef'] = True
        ref.hide_viewport = True     # out of every ray cast and triangle count until restore_shading
        SHADING_REFS[name] = ref
        out.append(dict(node=name, before=before, after=triangles(ob)))
    return out


def thin_inner_wall(keep):
    """Collapse the hidden inner wall of the solidified shell to `keep` of its faces.

    The shell is built as two nested surfaces 4 mm apart; the inner one (VehicleInterior) is only
    ever seen through the glass as a dark lining. Vertices touching any other material - the
    paint surface, the rim of every panel edge, the trim - are held in place by the decimator's
    vertex-group weighting, so the silhouette, the paint and every seam stay exactly as built.
    """
    report = []
    for name in SHELL:
        ob = bpy.data.objects.get(name)
        if ob is None:
            continue
        slots = {i for i, s in enumerate(ob.material_slots) if s.material and s.material.name == 'VehicleInterior'}
        if not slots:
            continue
        if name in CFG.get('thinSkip', ()):   # `thinSkip`: a deep scallop, where thinned chords bridge in front of the paint
            report.append(dict(node=name, skipped=True, before=triangles(ob)))
            continue
        me = ob.data
        before_total = triangles(ob)
        inner = sum(1 for t in me.loop_triangles if t.material_index in slots)
        outer_before = before_total - inner
        held = set()
        for poly in me.polygons:
            if poly.material_index not in slots:
                held.update(poly.vertices)
        group = ob.vertex_groups.new(name='HoldOuter')
        group.add(sorted(held), 1.0, 'REPLACE')
        target = outer_before + int(inner * keep)
        md = ob.modifiers.new('Thin inner wall', 'DECIMATE')
        md.ratio = target / before_total
        md.use_collapse_triangulate = True
        md.vertex_group = group.name
        md.invert_vertex_group = True
        md.vertex_group_factor = 1000
        bpy.context.view_layer.objects.active = ob
        bpy.ops.object.modifier_apply(modifier=md.name)
        ob.vertex_groups.remove(ob.vertex_groups['HoldOuter'])
        clamped = _keep_inner_wall_inside(ob, slots)
        after_total = triangles(ob)
        outer_after = sum(1 for t in ob.data.loop_triangles if t.material_index not in slots)
        report.append(dict(node=name, before=before_total, after=after_total,
                           outerBefore=outer_before, outerAfter=outer_after, clampedInnerVerts=clamped))
    return report


def _keep_inner_wall_inside(ob, slots, depth=0.004, margin=0.001):
    """Put back inside the paint any inner-wall vertex the collapse moved through it.

    A collapse places each merged vertex at its quadric optimum, which on a thin two-wall sheet
    can land outside the outer wall: measured on the compactSuv, one merged roof-lining vertex
    stood 72 mm above the paint (a dark spike through the roof and a wrong bounding height). A
    vertex used only by inner faces is moved to `depth` behind the nearest outer surface whenever
    it is not at least `margin` behind it. Outer vertices are never touched.

    `innerWallBoxClamp` (deliveryVan only): the signed test reads the nearest outer face's normal,
    which misjudges a vertex whose nearest point is a face EDGE. On the deliveryVan one merged
    roof-lining vertex stood 246 mm above the roof, nearer the side wall's top edge than the roof,
    and read as inside. With the key set, a vertex outside the outer surface's bounding box is
    outside whatever the normal says: it is moved behind its nearest outer face and held inside
    the box. Other bodies keep the plain test, so their output does not change.
    """
    me = ob.data
    outer_polys = [p for p in me.polygons if p.material_index not in slots]
    if not outer_polys:
        return 0
    verts = [v.co.copy() for v in me.vertices]
    tree = BVHTree.FromPolygons(verts, [tuple(p.vertices) for p in outer_polys])
    outer_verts = {i for p in outer_polys for i in p.vertices}
    box_clamp = bool(CFG.get('innerWallBoxClamp'))
    if box_clamp:
        lo = [min(verts[i][k] for i in outer_verts) for k in range(3)]
        hi = [max(verts[i][k] for i in outer_verts) for k in range(3)]
    moved = 0
    for v in me.vertices:
        if v.index in outer_verts:
            continue
        outside = box_clamp and any(v.co[k] > hi[k] + margin or v.co[k] < lo[k] - margin for k in range(3))
        hit, normal, index, dist = tree.find_nearest(v.co, 10.0 if outside else 0.25)
        if hit is None:
            continue
        # face normals point out of the car; `behind` is how far inside the paint the vertex sits
        behind = (hit - v.co).dot(normal)
        if behind < margin or outside:
            co = hit - normal * depth
            if outside:
                co = Vector([min(hi[k] - depth, max(lo[k] + depth, co[k])) for k in range(3)])
            v.co = co
            moved += 1
    me.update()
    return moved


def seat_linings(facing=-0.3, inset=0.25):
    """Delete every inward-facing lining face that can be seen from outside the car.

    The door cards, the boot-lid lining and the thinned 4 mm inner wall face INTO the car. Where
    the build's thinning (its long collapsed triangles chord out through a curved wing: measured
    50 mm proud on the compactSuv) or the panel cut leaves one outside the outer skin (door-card
    faces up to 8 mm outside the sedan's Panel_FL), a single-sided material culls it, but the
    runtime shares one material per name across the fleet and the other bodies export these
    materials double-sided, so it draws as a dark patch on the paint. A lining face is dropped
    when a ray from its centre, or from any corner pulled `inset` toward the centre, leaves the car
    along the face's outward side (against its normal) without meeting anything. Faces behind the
    skin always meet it and stay; paint and outward-facing trim are never touched.
    """
    lining = {'VehicleInterior', 'VehicleTrimBlack'}
    paint_names = ('VehiclePaint',) + tuple(CFG.get('liningPaint', ()))   # `liningPaint`: a livery that is paint too
    dg = bpy.context.evaluated_depsgraph_get()
    scene = bpy.context.scene
    report = {}
    for name in SHELL:
        ob = bpy.data.objects.get(name)
        if ob is None:
            continue
        me = ob.data
        mats = [s.material.name if s.material else '' for s in ob.material_slots]
        paint = [p for p in me.polygons if mats[min(p.material_index, len(mats) - 1)] in paint_names]
        if not paint:
            continue
        tree = BVHTree.FromPolygons([v.co.copy() for v in me.vertices], [tuple(p.vertices) for p in paint])
        mw = ob.matrix_world
        n3 = mw.to_3x3().inverted().transposed()
        drop_faces = []
        for p in me.polygons:
            if mats[min(p.material_index, len(mats) - 1)] not in lining:
                continue
            hit, normal, _, _ = tree.find_nearest(p.center, 0.3)
            if hit is None or p.normal.dot(normal) >= facing:
                continue
            out = -(n3 @ p.normal).normalized()
            c = p.center
            samples = [c] + [c.lerp(me.vertices[i].co, 1.0 - inset) for i in p.vertices]
            for sp in samples:
                w = mw @ sp
                if not scene.ray_cast(dg, w + out * 0.001, out, distance=3.0)[0]:
                    drop_faces.append(p.index)
                    break
        if drop_faces:
            bm = bmesh.new()
            bm.from_mesh(me)
            bm.faces.ensure_lookup_table()
            bmesh.ops.delete(bm, geom=[bm.faces[i] for i in drop_faces], context='FACES')
            bm.to_mesh(me)
            bm.free()
            me.update()
        report[name] = len(drop_faces)
    return report


def drop_poking_linings(margin=0.0005):
    """`dropPokingLinings` bodies: delete inner-wall faces that stand in front of the paint.

    After the inner wall is thinned, a long collapsed inner-wall triangle across a concave stretch (a door scallop,
    a shoulder undercut) is a chord that stands IN FRONT of the paint although its vertices sit behind it: a dark
    patch on the paint. Every VehicleInterior face of the shell whose centre or any edge midpoint lies more than
    `margin` in front of the nearest paint face is deleted (the inner wall is only ever seen through the glass).
    """
    paint_names = ('VehiclePaint',) + tuple(CFG.get('liningPaint', ()))
    report = {}
    for name in SHELL:
        ob = bpy.data.objects.get(name)
        if ob is None:
            continue
        me = ob.data
        mats = [sl.material.name if sl.material else '' for sl in ob.material_slots]
        paint = [p for p in me.polygons if mats and mats[min(p.material_index, len(mats) - 1)] in paint_names]
        if not paint:
            continue
        tree = BVHTree.FromPolygons([v.co.copy() for v in me.vertices], [tuple(p.vertices) for p in paint])
        drop = []
        # `pokingMaterials` (wagon, fullSizeSuv): on an articulated panel the inner wall is already VehicleTrimBlack
        # (consolidate_articulated_materials runs first); a face of another listed material is dropped only when it
        # faces INTO the car (against the nearest paint normal), so outward-facing trim is never touched
        targets = tuple(CFG.get('pokingMaterials', ('VehicleInterior',)))
        for p in me.polygons:
            mname = mats[min(p.material_index, len(mats) - 1)] if mats else ''
            if mname not in targets:
                continue
            vs = [me.vertices[i].co for i in p.vertices]
            samples = [p.center] + [(vs[k] + vs[(k + 1) % len(vs)]) * 0.5 for k in range(len(vs))]
            if 'pokingMaterials' in CFG:
                # a long sliver chord along a door's curved front edge pokes out between its vertices, where the
                # nearest paint is the edge wall (normal along the car, so the nearest-point test reads 0): a
                # barycentric grid, each interior sample cast OUT of the car along the face's back side; a sample
                # within 2 cm of the skin whose outward ray meets no paint is in front of the paint
                grid = []
                for k in range(1, len(vs) - 1):
                    a_, b_, c_ = vs[0], vs[k], vs[k + 1]
                    grid += [a_ * (1 - (i + j) / 16) + b_ * (i / 16) + c_ * (j / 16)
                             for i in range(1, 16) for j in range(1, 16 - i)]
                out_ = -p.normal

                def _pokes(g_):
                    h_, _, _, d_ = tree.find_nearest(g_, 0.02)
                    return h_ is not None and d_ > margin and tree.ray_cast(g_ + out_ * 1e-4, out_, 0.05)[0] is None
                if any(_pokes(g_) for g_ in grid):
                    if mname == 'VehicleInterior' or p.normal.dot((tree.find_nearest(p.center, 0.1)[1] or out_)) < -0.5:
                        drop.append(p.index)
                        continue
            if mname != 'VehicleInterior':
                h0, n0, _, _ = tree.find_nearest(p.center, 0.1)
                if h0 is None or p.normal.dot(n0) > -0.5:
                    continue
            for sp in samples:
                h, n, _, _ = tree.find_nearest(sp, 0.1)
                if h is not None and (sp - h).dot(n) > margin:
                    drop.append(p.index)
                    break
        if drop:
            bm = bmesh.new()
            bm.from_mesh(me)
            bm.faces.ensure_lookup_table()
            bmesh.ops.delete(bm, geom=[bm.faces[i] for i in drop], context='FACES')
            bm.to_mesh(me)
            bm.free()
            me.update()
        report[name] = len(drop)
    return report


def seat_arch_lips(wall=0.004):
    """Keep each arch-lip bead out of the wheel well.

    The skill sinks a round bead into the flank along every arch edge; the part of the tube below
    the paint is hidden inside the 4 mm wall, but its inboard side also hangs into the well, where
    a lifted, steered tyre sweeps. Every bead vertex inboard of the wall's inner face (measured on
    the shell at that vertex's own height and station) is pressed onto that face. Nothing visible
    moves: the bead's outer half is untouched.
    """
    body = bpy.data.objects['Body']
    dg = bpy.context.evaluated_depsgraph_get()
    tree = BVHTree.FromObject(body, dg)
    inv = body.matrix_world.inverted()
    out = {}
    for ob in meshes():
        if not ob.name.startswith(('ArchLipF', 'ArchLipR')):
            continue
        mw = ob.matrix_world
        mi = mw.inverted()
        moved = 0
        for v in ob.data.vertices:
            w = mw @ v.co
            side = 1.0 if w.x > 0 else -1.0
            hit = tree.ray_cast(inv @ Vector((side * 2.0, w.y, w.z)), inv.to_3x3() @ Vector((-side, 0.0, 0.0)), 3.0)[0]
            if hit is None:
                continue
            face = abs((body.matrix_world @ hit).x) - wall
            if abs(w.x) < face:
                w.x = side * face
                v.co = mi @ w
                moved += 1
        ob.data.update()
        out[ob.name] = moved
    return out


# --------------------------------------------------------------------------- tiers

def prepare_near(args, planar=False):
    bpy.ops.wm.open_mainfile(filepath=str(args.blend))
    removed, keys = drop_stage_and_keys()
    mats = rename_materials()
    flattened = flatten_transmission()
    names = rename_nodes()
    hinges = rewrite_hinges()
    plates = add_plates()
    lips = seat_arch_lips()
    planar_report = planar_shell(CFG['planarDeg']) if planar else None
    inner = thin_inner_wall(args.inner_keep)
    detached = detach_lid_static_parts()
    panels = consolidate_articulated_materials()   # after the thinning, which keys on VehicleInterior
    linings = seat_linings(facing=CFG.get('liningFacing', -0.3))
    if CFG.get('dropPokingLinings'):
        linings['poking'] = drop_poking_linings()
    shading = restore_shading() if planar else None
    bpy.context.scene[FRAME_KEY] = FRAME
    log(stage='near', stageRemoved=len(removed), shapeKeysDropped=keys, materials=mats, renamed=len(names), articulated=panels,
        hinges=hinges, plates=plates, archLips=lips, planar=planar_report, innerWall=inner,
        transmissionFlattened=flattened, linings=linings)
    return dict(stageRemoved=len(removed), shapeKeysDropped=keys, materials=mats, hinges=hinges,
                plates=plates, archLips=lips, planar=planar_report, innerWall=inner, articulated=panels,
                transmissionFlattened=flattened)


def budget_near():
    """Near tier to the cap: glass and seals reduced with protected boundaries only if needed."""
    reductions = []
    over = total_triangles() - CAPS[0]
    if over > 0:
        for ob in sorted((o for o in meshes() if o.name.endswith('_Seal')), key=triangles, reverse=True):
            if over <= 0:
                break
            cur = triangles(ob)
            target = max(int(cur * 0.55), cur - over)
            reductions.append(reduce_protected(ob, target, False))
            over = total_triangles() - CAPS[0]
    if over > 0:
        # recess housings: the dark cups behind grilles, intakes and lamps; boundaries held so each
        # still closes its cut-out exactly
        for ob in sorted((o for o in meshes() if o.name.endswith('_Housing') and not o.name.startswith('Mirror')),
                         key=triangles, reverse=True):
            if over <= 0:
                break
            cur = triangles(ob)
            reductions.append(reduce_protected(ob, max(int(cur * 0.6), cur - over), True))
            over = total_triangles() - CAPS[0]
    if over > 0:
        glass = [o for o in meshes() if o.name in ('Windscreen', 'RearWindow') or o.name.startswith(('Window_', 'Glass_Quarter'))]
        # `nearGlassSkip` (cityBus): panes left whole at LOD0 (halving the bus door windows folded the pane under its
        # frit ribbon and drew dark double-glass wedges in the door leaves)
        glass = [o for o in glass if not o.name.startswith(tuple(CFG.get('nearGlassSkip', ())))]
        for ob in sorted(glass, key=triangles, reverse=True):
            if over <= 0:
                break
            cur = triangles(ob)
            reductions.append(reduce_protected(ob, max(int(cur * 0.5), cur - over), True))
            over = total_triangles() - CAPS[0]
    if over > 0 and CFG.get('nearWheelShare'):
        # the 20-inch split-spoke wheels carry 62k triangles of this build: tyres and rims share the rest
        # of the overage with protected boundaries (the tyre revolve and the spoke lip keep their silhouettes)
        wheels = [o for o in meshes() if o.name.startswith('Wheel_') and o.name.endswith(('_Tyre', '_Lip'))]
        pool = sum(triangles(o) for o in wheels)
        for ob in sorted(wheels, key=triangles, reverse=True):
            cur = triangles(ob)
            share = int(over * cur / pool) + 1
            reductions.append(reduce_protected(ob, max(int(cur * (1 - CFG['nearWheelShare'])), cur - share), True))
        over = total_triangles() - CAPS[0]
    if over > 0 and CFG.get('nearTrimCand'):
        # tyres, rims and the chrome trim tubes (smooth revolves and sweeps that lose nothing
        # visible at 60%), boundaries held
        cand = [o for o in meshes() if o.name.endswith(('_Tyre', '_Lip')) or o.name.startswith(tuple(CFG['nearTrimCand']))]
        for ob in sorted(cand, key=triangles, reverse=True):
            if over <= 0:
                break
            cur = triangles(ob)
            reductions.append(reduce_protected(ob, max(int(cur * 0.6), cur - over), True))
            over = total_triangles() - CAPS[0]
    if over > 0:
        # still over after seals/housings/glass/wheels: the dense detail meshes (the subdivided
        # mirror caps, seats, engine, lamp parts, steering wheel) share the rest, protected boundaries, each kept >= 50%
        pool = [o for o in meshes() if o.name.startswith(('Mirror_', 'Seat', 'Engine', 'Lamp_', 'SteeringWheel', 'Dash', 'Console', 'Fan_')
                                                         + tuple(CFG.get('nearPool', ())))
                and triangles(o) > 200 and not o.name.startswith(tuple(CFG.get('nearKeepWhole', ())))]
        # up to three passes (a protected reduction can stop short of its target); a body that was
        # under the cap after the first pass is unchanged, and one still over used to fail here
        for pass_ in range(CFG.get("nearPasses", 3)):   # nearPasses
            if over <= 0:
                break
            tot = sum(triangles(o) for o in pool) or 1
            for ob in sorted(pool, key=triangles, reverse=True):
                cur = triangles(ob)
                share = int(over * (1.15 + 0.35 * pass_) * cur / tot) + 1
                reductions.append(reduce_protected(ob, max(int(cur * 0.5), cur - share), True))
            over = total_triangles() - CAPS[0]
    if total_triangles() > CAPS[0]:
        raise ValueError(f'near tier over its cap: {total_triangles()} > {CAPS[0]}')
    return reductions


def drop(pred):
    gone = []
    for ob in list(meshes()):
        if pred(ob):
            gone.append(ob.name)
            bpy.data.objects.remove(ob, do_unlink=True)
    return gone


def strip_interior(names):
    """Delete the inner-wall faces of `names` (far tier: the far glass is opaque)."""
    for name in names:
        ob = bpy.data.objects.get(name)
        if ob is None:
            continue
        slots = {i for i, s in enumerate(ob.material_slots) if s.material and s.material.name == 'VehicleInterior'}
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        bmesh.ops.delete(bm, geom=[f for f in bm.faces if f.material_index in slots], context='FACES')
        bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
        bm.to_mesh(ob.data)
        bm.free()


def allocate(table, cap, body_floor):
    """Reduce every part to its table target, then give Body what is left under the cap."""
    reductions = []
    body = bpy.data.objects['Body']
    for ob in list(meshes()):
        if ob is body:
            continue
        target, protect = table(ob)
        if target is not None and triangles(ob) > target:
            reductions.append(reduce_protected(ob, target, protect))
    rest = cap - sum(triangles(o) for o in meshes() if o is not body) - 8
    if rest < body_floor:
        census = sorted(((o.name, triangles(o)) for o in meshes() if o is not body), key=lambda r: -r[1])[:20]
        raise ValueError(f'parts leave {rest} triangles for the body (floor {body_floor}); largest: {census}')
    reductions.append(reduce_protected(body, rest, True))
    if total_triangles() > cap:
        # boundary back-off may overshoot: take the remainder from the body, unprotected
        reductions.append(reduce_protected(body, triangles(body) - (total_triangles() - cap), False))
    if total_triangles() > cap:
        raise ValueError(f'tier over its cap: {total_triangles()} > {cap}')
    return reductions


def rebuild_rim(major, minor):
    """Replace SteeringWheel_Rim with a `major` x `minor` torus fitted to the built rim.

    Centre, plane and radii are measured from the rim's own vertices (mean, least-variance axis,
    mean ring and tube radii), in the rim node's own frame; node, parent and material are kept.
    """
    ob = bpy.data.objects.get('SteeringWheel_Rim')
    if ob is None:
        return None
    pts = [v.co.copy() for v in ob.data.vertices]
    c = sum(pts, Vector()) / len(pts)
    cov = [[sum((p - c)[i] * (p - c)[j] for p in pts) for j in range(3)] for i in range(3)]
    axis = min((Vector(e) for e in _eigenvectors(cov)), key=lambda e: sum(((p - c).dot(e)) ** 2 for p in pts))
    axis.normalize()
    u = (Vector((1, 0, 0)) - axis * axis.x)
    if u.length < 1e-6:
        u = Vector((0, 1, 0)) - axis * axis.y
    u.normalize()
    w = axis.cross(u)
    radial = [((p - c) - axis * (p - c).dot(axis)).length for p in pts]
    R = sum(radial) / len(radial)
    r = sum(math.hypot(rad - R, (p - c).dot(axis)) for rad, p in zip(radial, pts)) / len(pts)
    verts, faces = [], []
    for i in range(major):
        a = 2 * math.pi * i / major
        d = u * math.cos(a) + w * math.sin(a)
        for j in range(minor):
            b = 2 * math.pi * j / minor
            verts.append(c + d * (R + r * math.cos(b)) + axis * (r * math.sin(b)))
    for i in range(major):
        for j in range(minor):
            i2, j2 = (i + 1) % major, (j + 1) % minor
            faces.append((i * minor + j, i2 * minor + j, i2 * minor + j2, i * minor + j2))
    me = bpy.data.meshes.new('SteeringWheel_Rim-mid')
    me.from_pydata([tuple(v) for v in verts], [], faces)
    for m in ob.data.materials:
        me.materials.append(m)
    for f in me.polygons:
        f.use_smooth = True
    old = ob.data
    ob.data = me
    if old.users == 0:
        bpy.data.meshes.remove(old)
    return dict(radiusM=round(R, 4), tubeM=round(r, 4), triangles=triangles(ob))


def _eigenvectors(m):
    """Eigenvectors of a symmetric 3x3 matrix by Jacobi rotation (no numpy dependency)."""
    a = [row[:] for row in m]
    v = [[1.0 if i == j else 0.0 for j in range(3)] for i in range(3)]
    for _ in range(50):
        p, q = max(((i, j) for i in range(3) for j in range(i + 1, 3)), key=lambda t: abs(a[t[0]][t[1]]))
        if abs(a[p][q]) < 1e-15:
            break
        theta = 0.5 * math.atan2(2 * a[p][q], a[q][q] - a[p][p])
        cth, sth = math.cos(theta), math.sin(theta)
        for k in range(3):
            akp, akq = a[k][p], a[k][q]
            a[k][p], a[k][q] = cth * akp - sth * akq, sth * akp + cth * akq
        for k in range(3):
            apk, aqk = a[p][k], a[q][k]
            a[p][k], a[q][k] = cth * apk - sth * aqk, sth * apk + cth * aqk
        for k in range(3):
            vkp, vkq = v[k][p], v[k][q]
            v[k][p], v[k][q] = cth * vkp - sth * vkq, sth * vkp + cth * vkq
    return [[v[0][i], v[1][i], v[2][i]] for i in range(3)]


def planar_skins(angle_deg, with_body=False):
    """Merge the panel skins (and optionally the body shell) by angle, material zones and seams kept."""
    out = []
    for name in SHELL:
        if (name == 'Body' and not with_body) or bpy.data.objects.get(name) is None:
            continue
        ob = bpy.data.objects[name]
        before = triangles(ob)
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        bmesh.ops.dissolve_limit(bm, angle_limit=math.radians(angle_deg), use_dissolve_boundaries=False,
                                 verts=list(bm.verts), edges=list(bm.edges), delimit={'MATERIAL', 'SEAM'})
        bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if len(f.verts) > 3],
                              quad_method='BEAUTY', ngon_method='BEAUTY')
        bm.to_mesh(ob.data)
        bm.free()
        ob.data.update()
        ob['_planarDone'] = True
        out.append(dict(node=name, before=before, after=triangles(ob)))
    return out


def mid_engine_bay(ob):
    """The mid tier drops the engine bay, never the body panel that closes it.

    `Engine*` used to match `EngineCover_Skin` too, so the bus's rear engine lid (a body panel, not
    engine-bay detail) vanished at LOD1 and the rear read as an open box at mid range. The cover keeps
    its `midByName` budget; its hinges, latches and louvres still go through the body's `midDrop`."""
    return ob.name.startswith('Engine') and not ob.name.startswith('EngineCover')


def mid_table(ob):
    n = ob.name
    if n in SIGN_NODES:
        return None, False   # the sign boxes are a few dozen faces; never reduce them
    if ob.get('_planarDone'):
        return None, False   # merged by angle in planar_skins
    if n == 'SteeringWheel_Rim' and CFG.get('midRim'):
        return None, False   # rebuilt as a torus before the allocation
    if n in (CFG.get('midByName') or {}):
        return tuple(CFG['midByName'][n])   # big non-cage parts (the dry-van shell, bumper)
    if n.startswith('Panel_'):
        return (700 if n[6] == 'F' else 600), True
    if n == 'HoodSkin':
        return 700, True
    if n == 'TrunkSkin':
        return 600, True
    if n.startswith('Wheel_'):
        role = n.split('_', 2)[-1]
        return {'Tyre': 256, 'Lip': 180, 'Disc': 36, 'Caliper': 24}.get(role, 24), False
    if n in ('Windscreen', 'RearWindow'):
        return 200, True
    if n.startswith(('Window_', 'Glass_Quarter')):
        return 80, True
    if n.endswith('_Seal'):
        return 80, False
    if n.startswith('Lamp_'):
        return (110 if n.count('_') == 2 else 48), False
    if n.startswith(('Seat', 'Dash', 'Console', 'Steering', 'Screen', 'Cabin', 'FloorPan', 'DoorCard')):
        return 24, False
    if n.startswith(('Arch_', 'Intake', 'Grille', 'Exhaust', 'Plate')):
        return 60, False
    if n.startswith('Mirror'):
        return 40, False
    return min(triangles(ob), 40), False


def far_table(ob):
    n = ob.name
    if ob.get('_farTyre'):
        return None, False   # `farTyreSegments`: rebuilt by far_tyres at its own segment count
    if n in SIGN_NODES:
        return None, False
    skin = CFG.get('farSkinTargets') or {}
    key = 'Panel' if n.startswith('Panel_') else n
    if key in skin:
        return skin[key], False
    for k_, v_ in skin.items():          # prefix rules (`farSkinTargets` keys ending in '_': the bus skirt flaps, door leaves)
        if k_.endswith('_') and n.startswith(k_):
            return v_, False
    if n.startswith('Panel_'):
        return 110, False
    if n == 'HoodSkin':
        return 90, False
    if n == 'TrunkSkin':
        return 110, False
    if n.startswith('Wheel_'):
        role = n.split('_', 2)[-1]
        return {'Tyre': 160, 'Face': 32}.get(role, 8), False
    if n in ('Windscreen', 'RearWindow'):
        return 60, False
    if n.startswith(('Window_', 'Glass_Quarter')):
        return 20, False
    if (n in ('Lamp_Head_L', 'Lamp_Head_R', 'Lamp_Tail_L', 'Lamp_Tail_R', 'Lamp_TailInner_L', 'Lamp_TailInner_R')
            or n in CFG.get('tailLampNodes', ())):
        return 40, False
    if n.startswith('Lamp_'):
        return 16, False
    if n.startswith(('Grille_Front_Housing', 'Intake_', 'Exhaust_')):
        return 24, False
    return min(triangles(ob), 12), False


FAR_KEEP = ('Body', 'HoodSkin', 'TrunkSkin', 'Windscreen', 'RearWindow', 'PlateFront', 'PlateRear')


def far_keep(ob):
    n = ob.name
    if CFG.get('farDrop') and (n.startswith(tuple(CFG['farDrop'])) or n.endswith(tuple(CFG.get('farDropSuffix', ())))):
        return False   # parts a 40-240 m view cannot resolve on this body
    if n in SIGN_NODES:
        return True
    if n in FAR_KEEP or n.startswith(('Panel_', 'Window_', 'Glass_Quarter', 'Arch_F_', 'Arch_R_')):
        return True
    if n.startswith('Wheel_') and n.split('_', 2)[-1] in ('Tyre', 'Lip'):
        return True   # the Lip is swapped for a flat face by far_wheel_faces
    if CFG.get('farKeepExtra') and n.startswith(tuple(CFG['farKeepExtra'])):
        return True   # the dry-van box, bumper, chassis masses and the inner duals
    if n.startswith('Lamp_') and not n.endswith(('Reflector0', 'Reflector1')):
        return True
    if n.startswith(('Grille_Front_Housing', 'Intake_Front_L_Housing', 'Intake_Front_R_Housing',
                     'Intake_Lower_Housing', 'Mirror_L_Housing', 'Mirror_R_Housing', 'Plate_Rear_Housing')):
        return True
    return False


def clamp_tyre_radius():
    """Hold every tyre vertex on or inside the rolling radius (`tyreRadiusClamp` bodies, LOD1/LOD2).

    A collapse places merged vertices at their quadric optimum, which on a tyre's rounded shoulder
    lies outside the tread: the drawn wheel grows and sinks into the road. Each tyre vertex is
    measured from its wheel's spin pivot, perpendicular to the spin axis, and pulled back to the
    radius when it lies beyond it. Faces and triangle counts do not change.
    """
    radius = CFG['wheelRadiusM']
    moved = {}
    for corner in ('FL', 'FR', 'RL', 'RR'):
        spin = bpy.data.objects.get(f'Wheel_{corner}_Spin') or bpy.data.objects.get(f'Wheel_{corner}')
        if spin is None:
            continue
        pivot = spin.matrix_world.translation.copy()
        axis = (spin.matrix_world.to_3x3() @ Vector((1.0, 0.0, 0.0))).normalized()
        for ob in [o for o in spin.children_recursive if o.type == 'MESH' and 'Tyre' in o.name]:
            mw, inv = ob.matrix_world, ob.matrix_world.inverted()
            n = 0
            for v in ob.data.vertices:
                w = mw @ v.co
                d = w - pivot
                radial = d - axis * d.dot(axis)
                if radial.length > radius:
                    v.co = inv @ (pivot + axis * d.dot(axis) + radial * (radius / radial.length))
                    n += 1
            ob.data.update()
            if n:
                moved[ob.name] = n
    log(stage='tyreRadiusClamp', moved=moved)
    return moved


def far_wheel_faces(segments=16):
    """Replace each far rim with a flat alloy face.

    Ten twin spokes cannot survive a few dozen triangles - a decimated rim is a scribble inside the
    tyre. At the far tier the rim reads as a light disc, which is what the authored fleet draws
    there (`Wheel_*_Face`). The disc sits at the rim's own outboard face and radius, measured in
    the spin pivot's frame, and rides on the spin pivot like the rim did.
    """
    out = {}
    for c in ('FL', 'FR', 'RL', 'RR'):
        lip = bpy.data.objects.get(f'Wheel_{c}_Lip')
        spin = bpy.data.objects.get(f'Wheel_{c}_Spin')
        if lip is None or spin is None:
            continue
        m = spin.matrix_world.inverted() @ lip.matrix_world
        pts = [m @ v.co for v in lip.data.vertices]
        outboard = max(pts, key=lambda p: abs(p.x)).x
        radius = max((p.y * p.y + p.z * p.z) ** 0.5 for p in pts) * 0.97
        side = 1.0 if outboard > 0 else -1.0
        x = outboard - side * 0.004
        verts = [(x, 0.0, 0.0)] + [(x, radius * math.cos(2 * math.pi * k / segments), radius * math.sin(2 * math.pi * k / segments))
                                   for k in range(segments)]
        faces = [(0, 1 + k, 1 + (k + 1) % segments) if side > 0 else (0, 1 + (k + 1) % segments, 1 + k)
                 for k in range(segments)]
        me = bpy.data.meshes.new(f'Wheel_{c}_Face')
        me.from_pydata(verts, [], faces)
        me.materials.append(lip.data.materials[0])
        ob = bpy.data.objects.new(f'Wheel_{c}_Face', me)
        bpy.context.scene.collection.objects.link(ob)
        ob.parent = spin
        ob.matrix_parent_inverse.identity()
        ob.matrix_basis.identity()
        bpy.data.objects.remove(lip, do_unlink=True)
        out[c] = dict(radiusM=round(radius, 4), outboardM=round(x, 4))
    return out


def frit_merge(angle_deg):
    """`tierFritMerge`: the ray-cast frit ribbons (8 mm quads round every pane, panels._frit) are merged
    by angle at the mid and far tiers. On the bus's large panes they carried ~1000 triangles per door window, the
    reducer cannot thin a boundary-protected glass part, and LOD1 ran out of budget for the body. The ribbon is its own
    geometry (not welded to the pane), so only the ribbon's VehicleTrimBlack faces on glass nodes dissolve; on straight runs a ribbon collapses to a
    few long quads, round the corners it keeps its arcs. Returns {node: [before, after]}."""
    out = {}
    for ob in meshes():
        names = [sl.material.name if sl.material else '' for sl in ob.material_slots]
        # the skill's Frit leaves as VehicleTrimBlack (MATERIALS); only glass nodes carry a frit ribbon
        if not any(n in GLASS for n in names) or 'VehicleTrimBlack' not in names:
            continue
        slots = {k for k, n in enumerate(names) if n == 'VehicleTrimBlack'}   # a door window can carry the slot twice
        before = triangles(ob)
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        fr = [f for f in bm.faces if f.material_index in slots]
        fs = set(fr)
        dv = [v for v in bm.verts if v.link_faces and all(f in fs for f in v.link_faces)]
        de = [e for e in bm.edges if e.link_faces and all(f in fs for f in e.link_faces)]
        bmesh.ops.dissolve_limit(bm, angle_limit=math.radians(angle_deg), use_dissolve_boundaries=False,
                                 verts=dv, edges=de, delimit={'MATERIAL'})
        bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if len(f.verts) > 3 and f.material_index in slots],
                              quad_method='BEAUTY', ngon_method='BEAUTY')
        # a ribbon on a curved pane (the front door leaves round the nose corner) does not merge by angle; past
        # `tierFritMax` triangles it is dropped at this tier (a 12 mm band does not resolve at mid / far range)
        left = [f for f in bm.faces if f.material_index in slots]
        dropped = False
        if len(left) > CFG.get('tierFritMax', 10 ** 9):
            bmesh.ops.delete(bm, geom=left, context='FACES')
            bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
            dropped = True
        bm.to_mesh(ob.data)
        bm.free()
        ob.data.update()
        out[ob.name] = [before, triangles(ob)] + (['ribbon dropped'] if dropped else [])
    log(stage='fritMerge', merged=out)
    return out


def far_flat(prefixes):
    """Flat-shade the far tier's flat panels (`farFlatPrefixes`); returns the node names."""
    out = []
    for ob in [o for o in meshes() if o.name.startswith(tuple(prefixes))]:
        ob.data.polygons.foreach_set('use_smooth', [False] * len(ob.data.polygons))
        if hasattr(ob.data, 'set_sharp_from_angle'):
            ob.data.set_sharp_from_angle(angle=0.0)
        ob.data.update()
        out.append(ob.name)
    return out


def far_tyres(segments):
    """`farTyreSegments`: each far tyre (Wheel_*_Tyre, Wheel_*_TyreInner) is rebuilt as a clean lathe
    of `segments` sides instead of being decimated. The 48-segment build tyre collapsed to ~8 sides at the far budget
    (160 tris; the inner duals to 8) and drew octagons at range. The lathe is measured in the spin pivot's frame (axle
    = x, as in far_wheel_faces): outboard face ring (inner radius -> shoulder), a chamfered shoulder, the tread and the
    inboard face, 4 strips x segments x 2 tris. Node, parent and material are kept; far_table leaves it as built."""
    out = {}
    for ob in [o for o in meshes() if o.name.startswith('Wheel_') and o.name.endswith(('_Tyre', '_TyreInner'))]:
        c = ob.name.split('_')[1]
        spin = bpy.data.objects.get(f'Wheel_{c}_Spin')
        if spin is None or not ob.data.vertices:
            continue
        m = spin.matrix_world.inverted() @ ob.matrix_world
        pts = [m @ v.co for v in ob.data.vertices]
        rr = [(p.y * p.y + p.z * p.z) ** 0.5 for p in pts]
        ro, ri = max(rr), min(rr)
        xs = [p.x for p in pts]
        x0, x1 = min(xs), max(xs)
        side = 1.0 if abs(x1) > abs(x0) else -1.0
        xo, xi = (x1, x0) if side > 0 else (x0, x1)
        ch = min(0.04, 0.25 * (ro - ri), 0.25 * (x1 - x0))
        prof = [(xo, ri), (xo, ro - ch), (xo - side * ch, ro), (xi, ro), (xi, ri)]
        mi = m.inverted()
        verts, faces = [], []
        for k in range(segments):
            a = 2 * math.pi * k / segments
            for x, r in prof:
                verts.append(tuple(mi @ Vector((x, r * math.cos(a), r * math.sin(a)))))
        P = len(prof)
        for k in range(segments):
            k2 = (k + 1) % segments
            for j in range(P - 1):
                q = (k * P + j, k * P + j + 1, k2 * P + j + 1, k2 * P + j)
                faces.append(q if side > 0 else q[::-1])
        old = ob.data
        me = bpy.data.meshes.new(old.name + '_far')
        me.from_pydata(verts, [], faces)
        me.update()
        for mat in old.materials:
            me.materials.append(mat)
        me.polygons.foreach_set('use_smooth', [True] * len(me.polygons))
        before = triangles(ob)
        ob.data = me
        if m.determinant() < 0:   # winding above faces outward in a right-handed spin frame
            me.flip_normals()
        ob['_farTyre'] = True
        out[ob.name] = dict(before=before, after=triangles(ob), radiusM=round(ro, 4), innerM=round(ri, 4))
    log(stage='farTyres', tyres=out)
    return out


# --------------------------------------------------------------------------- cage-built tiers
#
# The mid and far tiers are not a decimated copy of the dense build: they are the build's own
# hand-shaped cage (cage.json: a mirrored all-quad half cage), subdivided to the tier's level and
# fitted to the same limit surface the near tier was cut from. The paint, glass, lamp, grille and
# trim zones are carried over by projecting the near build's surfaces onto it, and the moving
# panels are split off by the same projection. Every tier is then the same faired surface, only
# coarser, so reflections stay smooth and the silhouette does not wander between tiers.

# near-build objects whose surface defines the zones the cage shell carries
ZONE_EXACT = ('Body', 'HoodSkin', 'TrunkSkin', 'Windscreen', 'RearWindow')
ZONE_PREFIX = ('Panel_', 'Window_', 'Glass_Quarter_')
ZONE_LENSES = {'Lamp_Head_L': 'VehicleHeadlamp', 'Lamp_Head_R': 'VehicleHeadlamp',
               'Lamp_Tail_L': 'VehicleTaillamp', 'Lamp_Tail_R': 'VehicleTaillamp',
               'Lamp_TailInner_L': 'VehicleTaillamp', 'Lamp_TailInner_R': 'VehicleTaillamp'}
# near-build parts the shell replaces (their areas become flat material zones on it)
SHELL_REPLACES = ('Windscreen', 'RearWindow', 'Window_', 'Glass_Quarter_', 'Lamp_', 'Grille_', 'Intake_',
                  'Plate_Front_Housing', 'Plate_Rear_Housing', 'Exhaust_L_Housing', 'Exhaust_R_Housing',
                  'FuelFlap', 'Badge', 'ArchMould', 'ArchLip', 'Handle_')
ZONE_MATERIAL_FOLD = {'VehicleInterior': 'VehicleTrimBlack', 'VehicleHeadlampLens': 'VehicleHeadlamp',
                      'VehicleTaillampLens': 'VehicleTaillamp', 'VehicleVoid': 'VehicleTrimBlack',
                      'VehicleMetalDark': 'VehicleTrimBlack'}


def _owner_of(ob):
    """(kind, corner) of the moving part a near-build object rides on; ('body', None) if static."""
    p = ob
    while p is not None:
        if p.name.startswith('SlideDoor_'):          # a sliding door translates
            return 'slide', p.name[len('SlideDoor_'):]
        if p.name.startswith('Door_'):
            return ('window' if ob.name == 'Window_' + p.name[5:] else 'door'), p.name[5:]
        if p.name in ('Hood', 'Trunk'):
            return p.name.lower(), None
        p = p.parent
    return 'body', None


def cage_mesh(path, level, name):
    """The hand-shaped half cage, mirrored and subdivided to `level` (creases honoured), applied."""
    data = json.loads(Path(path).read_text())
    me = bpy.data.meshes.new(name + '_cage')
    me.from_pydata([tuple(v) for v in data['verts']], [], [tuple(f) for f in data['faces']])
    me.update()
    if data.get('creases'):
        cr = me.edge_creases_ensure()
        lookup = {tuple(sorted(e.vertices)): e.index for e in me.edges}
        vals = [0.0] * len(me.edges)
        for a, b, w in data['creases']:
            i = lookup.get(tuple(sorted((a, b))))
            if i is not None:
                vals[i] = w
        cr.data.foreach_set('value', vals)
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    m = ob.modifiers.new('Mirror', 'MIRROR')
    m.use_axis = (True, False, False)
    m.use_clip = True
    m.use_mirror_merge = True
    m.merge_threshold = 1e-4
    if level > 0:
        s = ob.modifiers.new('Subdiv', 'SUBSURF')
        s.levels = s.render_levels = level
        s.quality = 3
        s.use_limit_surface = True
        s.use_creases = True
    dg = bpy.context.evaluated_depsgraph_get()
    baked = bpy.data.meshes.new_from_object(ob.evaluated_get(dg))
    ob.modifiers.clear()
    ob.data = baked
    bpy.data.meshes.remove(me)
    bm = bmesh.new()
    bm.from_mesh(baked)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(baked)
    bm.free()
    for f in baked.polygons:
        f.use_smooth = True
    return ob


def fit_to_limit(ob, target):
    """Move every vertex onto the nearest point of the fine `target` surface (the limit surface)."""
    md = ob.modifiers.new('limit fit', 'SHRINKWRAP')
    md.wrap_method = 'NEAREST_SURFACEPOINT'
    md.target = target
    bpy.context.view_layer.objects.active = ob
    bpy.ops.object.modifier_apply(modifier=md.name)


def _arch_cutter(a, side, stretch_band=0.12, segments=32):
    x0, x1 = a['x']
    lo, hi = (x0, x1) if side > 0 else (-x1, -x0)
    r, yc, zc = a['r'], a['y'], a['z']
    sy, slo = a.get('stretchY', 1.0), a.get('stretchLow', 1.0)
    bm = bmesh.new()
    ring_a, ring_b = [], []
    for k in range(segments):
        t = 2 * math.pi * k / segments
        y, z = r * math.cos(t) * sy, r * math.sin(t)
        if slo != 1.0:
            w = min(1.0, max(0.0, (stretch_band - z) / (2 * stretch_band)))
            w = w * w * (3 - 2 * w)
            y *= 1.0 + (slo - 1.0) * w
        ring_a.append(bm.verts.new((lo, yc + y, zc + z)))
        ring_b.append(bm.verts.new((hi, yc + y, zc + z)))
    for k in range(segments):
        k2 = (k + 1) % segments
        bm.faces.new((ring_a[k], ring_a[k2], ring_b[k2], ring_b[k]))
    bm.faces.new(list(reversed(ring_a)))
    bm.faces.new(ring_b)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    me = bpy.data.meshes.new('_arch_cut')
    bm.to_mesh(me)
    bm.free()
    cut = bpy.data.objects.new('_arch_cut', me)
    bpy.context.scene.collection.objects.link(cut)
    return cut


def cut_arches(ob, arches):
    """Open the wheel arches with the build's own arch cylinders (parts.json `arches`)."""
    for a in arches:
        for side in ((1, -1) if a.get('mirror', True) else (1,)):
            cut = _arch_cutter(a, side)
            md = ob.modifiers.new('arch', 'BOOLEAN')
            md.operation = 'DIFFERENCE'
            md.object = cut
            md.solver = 'EXACT'
            bpy.context.view_layer.objects.active = ob
            bpy.ops.object.modifier_apply(modifier=md.name)
            bpy.data.objects.remove(cut, do_unlink=True)
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if len(f.verts) > 4])
    bm.to_mesh(ob.data)
    bm.free()
    for f in ob.data.polygons:
        f.use_smooth = True


HEAD_LENSES = ('Lamp_Head_L', 'Lamp_Head_R')


def _limit_tree(target):
    """BVH over the triangulated limit surface plus its smooth vertex normals (world space)."""
    me = target.data
    me.calc_loop_triangles()
    mw = target.matrix_world
    n3 = mw.to_3x3().inverted().transposed()
    verts = [mw @ v.co for v in me.vertices]
    vn = [(n3 @ v.normal).normalized() for v in me.vertices]
    tris = [tuple(t.vertices) for t in me.loop_triangles]
    return dict(tree=BVHTree.FromPolygons(verts, tris), verts=verts, vn=vn, tris=tris)


def _limit_at(lim, p, reach):
    """(nearest point, interpolated smooth normal, distance) on the limit surface, or None."""
    hit, _, idx, dist = lim['tree'].find_nearest(p, reach)
    if hit is None:
        return None
    a, b, c = (lim['verts'][i] for i in lim['tris'][idx])
    w = mgeom.barycentric_transform(hit, a, b, c, Vector((1, 0, 0)), Vector((0, 1, 0)), Vector((0, 0, 1)))
    na, nb, nc = (lim['vn'][i] for i in lim['tris'][idx])
    n = na * w.x + nb * w.y + nc * w.z
    return hit, (n.normalized() if n.length > 1e-9 else na), dist


def _recess_prisms(parts, name):
    """The build's lamp outline for `name`: (plane, poly, range, mirrored) of its parts.json recess."""
    out = []
    for r in (parts or {}).get('recesses', []):
        base = r.get('name', '')
        mirrored = bool(r.get('mirror')) and base.endswith('_L') and base[:-2] + '_R' == name
        if (base == name or mirrored) and r.get('poly') and r.get('plane') and r.get('range'):
            out.append((r['plane'], r['poly'], r['range'], mirrored))
    return out


def _in_prisms(p, prisms, margin=0.012):
    if not prisms:
        return True   # a traced lamp has no prism; the skin test alone clamps it
    for plane, poly, rng, mirrored in prisms:
        x = -p.x if mirrored else p.x
        u, v, w = {'XZ': (x, p.z, p.y), 'YZ': (p.y, p.z, x), 'XY': (x, p.y, p.z)}[plane]
        if not (min(rng) - margin <= w <= max(rng) + margin):
            continue
        inside = False
        for (u0, v0), (u1, v1) in zip(poly, poly[1:] + poly[:1]):
            if (v0 > v) != (v1 > v) and u < u0 + (v - v0) * (u1 - u0) / (v1 - v0):
                inside = not inside
        if inside:
            return True
        for (u0, v0), (u1, v1) in zip(poly, poly[1:] + poly[:1]):
            du, dv = u1 - u0, v1 - v0
            t = max(0.0, min(1.0, ((u - u0) * du + (v - v0) * dv) / (du * du + dv * dv or 1e-12)))
            if math.hypot(u - u0 - t * du, v - v0 - t * dv) < margin:
                return True
    return False


def _through_material(ob, index, dg):
    """What the near build shows through a clear headlamp lens at one point."""
    n = ob.name
    if n.startswith('Lamp_Head') and any(k in n for k in ('_Core', '_Reflector', '_Projector')):
        return 'VehicleHeadlamp'      # the lit bowl, projector and bulb: the runtime drives these
    me = ob.evaluated_get(dg).data
    slots = [s.material.name if s.material else 'VehiclePaint' for s in ob.material_slots]
    mat = 'VehicleTrimBlack'
    if slots and index is not None and index < len(me.polygons):
        mat = slots[min(me.polygons[index].material_index, len(slots) - 1)]
    if mat in ('VehicleHeadlamp', 'VehicleDRL', 'VehicleIndicator'):
        return mat
    if n.endswith('_Housing'):
        return 'VehicleTrimBlack'     # the dark housing seen through the cover
    return ZONE_MATERIAL_FOLD.get(mat, mat)


def _head_lens_faces(ob, lim, prisms, skin=0.012):
    """{polygon index: material} for a headlamp lens: only faces ON the skin (within `skin` of the limit
    surface, facing out) and inside the lamp outline, each with the material seen through it."""
    scene = bpy.context.scene
    dg = bpy.context.evaluated_depsgraph_get()
    mw = ob.matrix_world
    n3 = mw.to_3x3().inverted().transposed()
    out = {}
    for poly in ob.data.polygons:
        c = mw @ poly.center
        nrm = (n3 @ poly.normal).normalized()
        at = _limit_at(lim, c, skin)
        if at is None or nrm.dot(at[1]) < 0.5 or not _in_prisms(c, prisms):
            continue
        o, d, mat = c - nrm * 0.001, -nrm, 'VehicleTrimBlack'
        for _ in range(8):
            ok, loc, _, idx, hob, _ = scene.ray_cast(dg, o, d, distance=0.3)
            if not ok:
                break
            if hob.name in HEAD_LENSES or hob.name.startswith('_limit'):
                o = loc + d * 0.0005
                continue
            mat = _through_material(hob, idx, dg)
            break
        out[poly.index] = mat
    return out


def zone_sources(target=None, parts=None):
    out = []
    lim = _limit_tree(target) if target is not None else None
    if target is not None:
        target.hide_viewport = True     # the look-through rays must not stop on the limit surface
        bpy.context.view_layer.update()
    for ob in meshes():
        n = ob.name
        if not (n in ZONE_EXACT or n.startswith(ZONE_PREFIX) or n in ZONE_LENSES
                or (n.endswith('_Housing') and not n.startswith(('Mirror', 'Exhaust'))) or n.startswith('Grille_Front')
                or (n.startswith('Intake_') and n.endswith('_Grille'))):
            continue
        # the OUTER surface only: the 4 mm inner wall (VehicleInterior) lies closer to a flat cage
        # face's centre than the curved paint does wherever the face sits inside the curvature
        slots = [s.material.name if s.material else 'VehiclePaint' for s in ob.material_slots]
        mw = ob.matrix_world
        verts = [mw @ v.co for v in ob.data.vertices]
        polys, mats = [], []
        face_mat = _head_lens_faces(ob, lim, _recess_prisms(parts, n)) if (n in HEAD_LENSES and lim) else None
        for poly in ob.data.polygons:
            if face_mat is not None:
                if poly.index in face_mat:
                    polys.append(tuple(poly.vertices))
                    mats.append(face_mat[poly.index])
                continue
            # an index past the last slot draws with the last slot (Blender and the glTF exporter
            # clamp it): the grille, intake and lamp housings carry index 1 over a single slot
            mat = slots[min(poly.material_index, len(slots) - 1)] if slots else 'VehiclePaint'
            if mat == 'VehicleInterior':
                continue
            polys.append(tuple(poly.vertices))
            mats.append(mat)
        if not polys:
            continue
        out.append(dict(ob=ob, tree=BVHTree.FromPolygons(verts, polys), mats=mats,
                        owner=_owner_of(ob), lens=None if face_mat is not None else ZONE_LENSES.get(n),
                        faceMat=face_mat))
    if target is not None:
        target.hide_viewport = False
        bpy.context.view_layer.update()
    heads = {m: sum(1 for s in out if s.get('faceMat') for v in s['faceMat'].values() if v == m)
             for m in ('VehicleHeadlamp', 'VehicleDRL', 'VehicleIndicator', 'VehicleTrimBlack')}
    log(stage='headLens', faces=heads)
    return out


def _zone_at(c, sources, reach, normal=None, agree=0.3, slack=0.015):
    """Material and owner of the nearest near-build surface. With `normal` (), a surface facing another way
    (dot < `agree`) is passed over for the nearest one that faces with the query within `slack` further: the boolean's
    long arch-cut slivers sample points a few mm from the lip, where the near build's wheel-well lining (facing into
    the well) is nearer than the flank paint, and a whole sliver up the flank used to turn VehicleUnderbody."""
    best = None
    for src in sources:
        hit = src['tree'].find_nearest(c, reach)
        if hit[0] is None:
            continue
        d = (hit[0] - c).length
        if normal is not None and hit[1].dot(normal) < agree:
            alt = [h for h in src['tree'].find_nearest_range(c, min(reach, d + slack)) if h[1].dot(normal) >= agree]
            if alt:
                h = min(alt, key=lambda h: h[3])
                hit, d = h, h[3]
            else:
                d += slack     # keep it only if nothing facing the right way is near
        if best is None or d < best[0]:
            best = (d, src, hit[2])
    if best is None:
        return ('VehiclePaint', ('body', None))
    _, src, index = best
    mat = src['lens'] or (src['mats'][index] if index is not None and index < len(src['mats']) else 'VehiclePaint')
    return (ZONE_MATERIAL_FOLD.get(mat, mat), src['owner'])


def project_zones(ob, sources, reach=0.12, strict=None):
    """Per face of `ob`: the material and owner the near build carries under most of that face.

    `strict`: materials a face takes only when EVERY sample agrees (else paint). Used for the
    features `feature_decals` redraws on top of the shell, so the shell's staircase of whole
    faces always stays inside the smooth decal instead of stepping out past its edge.

    Each face is sampled at its centre and half way to each corner; the majority wins (the centre
    breaks ties), so a zone edge lands on the face that is mostly inside it. Faces then left as a
    lone island among neighbours of one other zone are merged into their neighbours.
    """
    mw = ob.matrix_world
    n3 = mw.to_3x3().inverted().transposed()
    zones = []
    for poly in ob.data.polygons:
        c = mw @ poly.center
        fn = (n3 @ poly.normal).normalized()
        votes = [_zone_at(c, sources, reach, fn)]
        for vi in poly.vertices:
            votes.append(_zone_at(c.lerp(mw @ ob.data.vertices[vi].co, 0.5), sources, reach, fn))
        tally = {}
        for v in votes:
            tally[v] = tally.get(v, 0) + 1
        top = max(tally.values())
        win = votes[0] if tally[votes[0]] == top else max(tally, key=tally.get)
        if strict and win[0] in strict and tally[win] < len(votes):
            win = ('VehiclePaint', win[1])
        zones.append(win)
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bm.faces.ensure_lookup_table()
    neighbours = [[g.index for e in f.edges for g in e.link_faces if g.index != f.index] for f in bm.faces]
    bm.free()
    for _ in range(2):
        changed = 0
        for i, ns in enumerate(neighbours):
            if len(ns) < 3:
                continue
            around = [zones[j] for j in ns]
            if zones[i] in around:
                continue
            tally = {}
            for z in around:
                tally[z] = tally.get(z, 0) + 1
            best = max(tally, key=tally.get)
            if tally[best] * 2 > len(around):
                zones[i] = best
                changed += 1
        if not changed:
            break
    return zones


def refine_zone_edges(ob, sources, target, rounds=1, strict=None, only=None):
    """Split the faces on a zone or shut-line edge once more, on the limit surface.

    A zone edge on a level-1 cage runs as a staircase of whole faces; halving those faces - only
    those - halves the step without spending triangles on the smooth interior of each zone.
    Returns the re-projected zones.
    """
    zones = project_zones(ob, sources, strict=strict)
    for _ in range(rounds):
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        bm.faces.ensure_lookup_table()
        edge_faces = [f for f in bm.faces
                      if any(g is not f and zones[g.index] != zones[f.index]
                             and (only is None or only(zones[f.index]) or only(zones[g.index]))
                             for e in f.edges for g in e.link_faces)]
        edges = list({e for f in edge_faces for e in f.edges})
        bmesh.ops.subdivide_edges(bm, edges=edges, cuts=1, use_grid_fill=True)
        bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if len(f.verts) > 4])
        bm.to_mesh(ob.data)
        bm.free()
        for f in ob.data.polygons:
            f.use_smooth = True
        fit_to_limit(ob, target)
        zones = project_zones(ob, sources, strict=strict)
    return zones


def hold_moving_parts_inside(ob, zones, sources, reach=0.12):
    """A face rides on a moving panel only if every corner of it lies over that panel.

    A cage face that straddles a shut line would otherwise lift a piece of wing with the bonnet
    or a strip of sill with a door. Such a face stays on the body (its material is kept).
    """
    mw = ob.matrix_world
    family = lambda o: ('door', o[1]) if o[0] in ('door', 'window') else o
    cache = {}
    out = []
    for poly, (mat, owner) in zip(ob.data.polygons, zones):
        if owner[0] == 'body':
            out.append((mat, owner))
            continue
        ok = True
        for v in poly.vertices:
            if v not in cache:
                cache[v] = _zone_at(mw @ ob.data.vertices[v].co, sources, reach)[1]
            if family(cache[v]) != family(owner):
                ok = False
                break
        out.append((mat, owner) if ok else (mat, ('body', None)))
    return out


def assign_zones(ob, zones):
    names = sorted({m for m, _ in zones})
    ob.data.materials.clear()
    for n in names:
        mat = bpy.data.materials.get(n) or bpy.data.materials.new(n)
        ob.data.materials.append(mat)
    index = {n: i for i, n in enumerate(names)}
    for poly, (mat, _) in zip(ob.data.polygons, zones):
        poly.material_index = index[mat]


def split_owners(ob, zones, suffix=''):
    """Split `ob` by zone owner into the runtime's part nodes; returns {object name: triangles}."""
    groups = {}
    for i, (_, owner) in enumerate(zones):
        groups.setdefault(owner, []).append(i)
    made = {}
    for owner, faces in groups.items():
        kind, corner = owner
        if kind == 'body':
            name, parent = 'Body', None
        elif kind == 'door':
            name, parent = f'Panel_{corner}', bpy.data.objects[f'Door_{corner}']
        elif kind == 'window':
            name, parent = f'Window_{corner}', bpy.data.objects[f'Door_{corner}']
        elif kind == 'slide':
            name, parent = f'Panel_Slide{corner}', bpy.data.objects[f'SlideDoor_{corner}']
        else:
            name, parent = ('HoodSkin' if kind == 'hood' else 'TrunkSkin'), bpy.data.objects[kind.capitalize()]
        name += suffix
        keep = set(faces)
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        bm.faces.ensure_lookup_table()
        bmesh.ops.delete(bm, geom=[f for f in bm.faces if f.index not in keep], context='FACES')
        me = bpy.data.meshes.new(name)
        bm.to_mesh(me)
        bm.free()
        for m in ob.data.materials:
            me.materials.append(m)
        part = bpy.data.objects.new(name, me)
        bpy.context.scene.collection.objects.link(part)
        part.matrix_world = ob.matrix_world.copy()
        if parent is not None:
            part.parent = parent
            part.matrix_parent_inverse = parent.matrix_world.inverted()
        # drop material slots the piece does not use
        used = {p.material_index for p in me.polygons}
        for i in sorted(range(len(me.materials)), reverse=True):
            if i not in used:
                me.materials.pop(index=i)
        made[name] = triangles(part)
    bpy.data.objects.remove(ob, do_unlink=True)
    return made


def rebuild_trims(parts, shell_objects, prefixes, sides, step):
    """Rebuild the arch mouldings / beads (parts.json `sideTrims`) on the cage shell at `sides` x `step`.

    Same path, radius, offset and material the skill used, ray-cast along -X onto the tier's own
    shell, so the trim hugs the coarse surface exactly instead of floating off the dense one.
    """
    dg = bpy.context.evaluated_depsgraph_get()
    trees = [(o, BVHTree.FromObject(o, dg), o.matrix_world.inverted()) for o in shell_objects]
    made = {}
    for d in parts.get('sideTrims', []):
        if not d['name'].startswith(prefixes) or d.get('face', 'side') != 'side':
            continue
        mat = bpy.data.materials.get(material_names().get(d.get('material', 'Trim_Chrome'), d.get('material', '')))
        for side in ((1, -1) if d.get('mirror') else (1,)):
            pts = []
            path = d['path']
            # resample the (y, z) path uniformly at `step`, ends kept
            seg = [math.hypot(b[0] - a[0], b[1] - a[1]) for a, b in zip(path, path[1:])]
            total = sum(seg)
            count = max(2, int(round(total / step)) + 1)
            samples = []
            for k in range(count):
                s_ = total * k / (count - 1)
                i = 0
                while i < len(seg) - 1 and s_ > seg[i]:
                    s_ -= seg[i]
                    i += 1
                t = min(1.0, s_ / seg[i]) if seg[i] > 0 else 0.0
                samples.append((path[i][0] + (path[i + 1][0] - path[i][0]) * t, path[i][1] + (path[i + 1][1] - path[i][1]) * t))
            for y, z in samples:
                if True:
                    best = None
                    for o, tree, inv in trees:
                        o_ = inv @ Vector((side * 1.5, y, z))
                        dv = (inv.to_3x3() @ Vector((-side, 0, 0))).normalized()
                        hit = tree.ray_cast(o_, dv, 4.0)
                        if hit[0] is not None:
                            w = o.matrix_world @ hit[0]
                            if best is None or side * w.x > side * best[0].x:
                                nrm = (o.matrix_world.to_3x3() @ hit[1]).normalized()
                                best = (w, nrm)
                    if best is not None:
                        w, nrm = best
                        if nrm.x * side < 0:
                            nrm = -nrm
                        pts.append(w + nrm * d.get('offset', 0.004))
            if len(pts) < 2:
                continue
            name = f"{d['name']}{'_L' if side > 0 else '_R'}"
            radius = d.get('radius', 0.005)
            bm = bmesh.new()
            rings = []
            for i, p in enumerate(pts):
                tng = (pts[min(i + 1, len(pts) - 1)] - pts[max(i - 1, 0)]).normalized()
                ref = Vector((0, 0, 1)) if abs(tng.z) < 0.9 else Vector((1, 0, 0))
                u = tng.cross(ref).normalized()
                v = tng.cross(u).normalized()
                rings.append([bm.verts.new(p + (u * math.cos(2 * math.pi * k / sides) + v * math.sin(2 * math.pi * k / sides)) * radius)
                              for k in range(sides)])
            for a_, b_ in zip(rings, rings[1:]):
                for k in range(sides):
                    k2 = (k + 1) % sides
                    bm.faces.new((a_[k], a_[k2], b_[k2], b_[k]))
            bm.faces.new(list(reversed(rings[0])))
            bm.faces.new(rings[-1])
            bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
            me = bpy.data.meshes.new(name)
            bm.to_mesh(me)
            bm.free()
            if mat:
                me.materials.append(mat)
            for f in me.polygons:
                f.use_smooth = True
            ob = bpy.data.objects.new(name, me)
            bpy.context.scene.collection.objects.link(ob)
            made[name] = triangles(ob)
    return made



# Materials never redrawn as a decal: the shell itself carries paint and glass, the lining and the
# underbody are never seen from outside at these ranges.
DECAL_SKIP = {'VehiclePaint', 'VehicleGlass', 'VehicleInterior', 'VehicleUnderbody'}
LAMP_KEEP = {'VehicleHeadlamp', 'VehicleDRL', 'VehicleIndicator'}   # never collapsed out of a decal


def _decal_owner_name(owner):
    kind, corner = owner
    if kind == 'body':
        return 'Body_Trim', None
    if kind in ('door', 'window'):
        return f'Panel_{corner}_Trim', f'Door_{corner}'
    if kind == 'slide':
        return f'Panel_Slide{corner}_Trim', f'SlideDoor_{corner}'
    return ('HoodSkin_Trim' if kind == 'hood' else 'TrunkSkin_Trim'), kind.capitalize()


def extract_decals(sources, target, reach=0.10):
    """The near build's visible non-paint surfaces, as world-space pieces per moving part.

    Lamps, grilles, intakes, cladding, window surrounds and badges are cut out of the near build
    exactly as modelled: a face is kept when its material is a feature material, it faces out of
    the car, it lies within `reach` of the limit surface and a ray from it along its normal leaves
    the car without meeting any other part (so the reflector behind a lens, a grille bar's back
    and a door card never come along). Each vertex also records how far above the deepest point
    of its own part it sat, so a grille's bars stay in front of its dark backing once flattened.
    Returns {owner: dict(verts=[(co, lift)], faces=[(vi..., material)])}.
    """
    dg = bpy.context.evaluated_depsgraph_get()
    scene = bpy.context.scene
    ttree = BVHTree.FromObject(target, dg)
    target.hide_viewport = True      # the limit surface must not occlude the recessed features
    bpy.context.view_layer.update()
    dg = bpy.context.evaluated_depsgraph_get()
    out = {}
    for src in sources:
        ob = src['ob']
        # [exoticCoupe run] `decalSkipSuffix`: honeycomb cell walls (tens of thousands of faces) are not redrawn as decals;
        # their dark backing is. Merged into one Body_Trim with the rest, they took the whole decal budget and the
        # collapse tore every other feature (rear band, diffuser, side intakes) into dark slivers
        if ob.name.endswith(tuple(CFG.get('decalSkipSuffix', ()))):
            continue
        slots = [s.material.name if s.material else 'VehiclePaint' for s in ob.material_slots]
        mw = ob.matrix_world
        n3 = mw.to_3x3().inverted().transposed()
        me = ob.data
        wco = [mw @ v.co for v in me.vertices]
        # a grille or intake cup is deep: its dark backing sits well behind the opening
        deep = ob.name.endswith('_Housing') or 'Grille' in ob.name
        src_reach = 0.22 if deep else reach
        keep = []
        fm = src.get('faceMat')
        for poly in me.polygons:
            if fm is not None:
                if poly.index not in fm:
                    continue      # off the skin or outside the lamp outline
                mat = fm[poly.index]
            else:
                mat = slots[min(poly.material_index, len(slots) - 1)] if slots else 'VehiclePaint'
                mat = src['lens'] or mat
            mat = ZONE_MATERIAL_FOLD.get(mat, mat)
            if mat in DECAL_SKIP or mat in glass_names():
                continue
            c = mw @ poly.center
            n = (n3 @ poly.normal).normalized()
            hit, tn, _, dist = ttree.find_nearest(c, src_reach)
            if hit is None or n.dot(tn) < 0.2:
                continue
            ray = scene.ray_cast(dg, c + n * 0.002, n, distance=0.4)
            # the bars of a grille do not hide its backing: both are drawn, bars lifted in front
            if ray[0] and 'Grille' not in ray[4].name:
                continue
            keep.append((poly, mat))
        _dk = DECAL_KEPT.setdefault(ob.name, [0, 0])
        _dk[0] += len(keep); _dk[1] += len(me.polygons)
        if not keep:
            continue
        depth = {}
        for poly, _ in keep:
            for vi in poly.vertices:
                if vi not in depth:
                    hit, tn, _, _ = ttree.find_nearest(wco[vi], src_reach + 0.05)
                    depth[vi] = (wco[vi] - hit).dot(tn) if hit is not None else 0.0
        floor = min(depth.values())
        piece = out.setdefault(src['owner'], dict(verts=[], faces=[]))
        index = {}
        for poly, mat in keep:
            ids = []
            for vi in poly.vertices:
                if vi not in index:
                    index[vi] = len(piece['verts'])
                    piece['verts'].append((wco[vi].copy(), min(0.006, 0.2 * (depth[vi] - floor))))
                ids.append(index[vi])
            piece['faces'].append((tuple(ids), mat))
    target.hide_viewport = False
    bpy.context.view_layer.update()
    log(stage='decalSources', kept={k: v for k, v in DECAL_KEPT.items() if v[0]})
    DECAL_KEPT.clear()
    return out


DECAL_KEPT = {}   # [exoticCoupe run] per source: [faces kept as decals, faces] (diagnostic log only)


def place_decals(pieces, shells, offset, budget, articulated):
    """Lay each piece onto the tier's own shell `offset` (+ its kept relief) out along the shell
    normal, then reduce: planar merge that never crosses a material edge, and a collapse down to
    this piece's share of `budget` (by triangle count). Returns {object name: triangles}."""
    dg = bpy.context.evaluated_depsgraph_get()
    trees = [(o.matrix_world, o.matrix_world.inverted(), BVHTree.FromObject(o, dg)) for o in shells]

    def onto(co):
        best = None
        for mw, mi, tree in trees:
            hit, nrm, _, dist = tree.find_nearest(mi @ co, 0.3)
            if hit is not None and (best is None or dist < best[0]):
                best = (dist, mw @ hit, (mw.to_3x3() @ nrm).normalized())
        return best

    total = sum(len(p['faces']) for p in pieces.values()) or 1
    made = {}
    merged = {}
    for owner, piece in pieces.items():
        name, parent = _decal_owner_name(owner if articulated else ('body', None))
        merged.setdefault((name, parent), []).append(piece)
    for (name, parent), plist in merged.items():
        bm = bmesh.new()
        mats = []
        for piece in plist:
            verts = []
            for co, lift in piece['verts']:
                best = onto(co)
                if best is None:
                    verts.append(bm.verts.new(co))
                else:
                    verts.append(bm.verts.new(best[1] + best[2] * (offset + lift)))
            for ids, mat in piece['faces']:
                if len(set(ids)) < 3:
                    continue
                try:
                    f = bm.faces.new([verts[i] for i in ids])
                except ValueError:
                    continue
                if mat not in mats:
                    mats.append(mat)
                f.material_index = mats.index(mat)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
        me = bpy.data.meshes.new(name)
        bm.to_mesh(me)
        bm.free()
        for m in mats:
            me.materials.append(bpy.data.materials.get(m) or bpy.data.materials.new(m))
        ob = bpy.data.objects.new(name, me)
        bpy.context.scene.collection.objects.link(ob)
        bpy.context.view_layer.objects.active = ob
        md = ob.modifiers.new('merge flat', 'DECIMATE')
        md.decimate_type = 'DISSOLVE'
        md.angle_limit = math.radians(4.0)
        md.delimit = {'MATERIAL'}
        bpy.ops.object.modifier_apply(modifier=md.name)
        share = max(24, int(budget * sum(len(p['faces']) for p in plist) / total))
        if triangles(ob) > share:
            # the few lamp faces the runtime drives (a lit reflector, a DRL bar) are held by the collapse:
            # at the far budget it used to erase the sedan's only VehicleHeadlamp faces
            keep_ = LAMP_KEEP | set(CFG.get('lampKeepExtra', ()))     # [exoticCoupe run] tail lamps held too (LOD2 jagged)
            lamp_slots = {i for i, m in enumerate(ob.data.materials) if m and m.name in keep_}
            held = sorted({v for pl in ob.data.polygons if pl.material_index in lamp_slots for v in pl.vertices})
            md = ob.modifiers.new('reduce', 'DECIMATE')
            md.ratio = share / triangles(ob)
            md.use_collapse_triangulate = True
            if held:
                grp = ob.vertex_groups.new(name='KeepLamp')
                grp.add(held, 1.0, 'REPLACE')
                md.vertex_group = grp.name
                md.invert_vertex_group = True
                md.vertex_group_factor = 1000
            bpy.ops.object.modifier_apply(modifier=md.name)
            if held:
                ob.vertex_groups.remove(ob.vertex_groups['KeepLamp'])
        if CFG.get('decalBoxClamp'):
            # `decalBoxClamp` (deliveryVan): the collapse places merged vertices at their quadric optimum; on the van's
            # far tier one landed 185 mm ahead of the nose and 0.3 m under the bumper, and its fan drew a 1.9 m
            # TrimBlack wedge. A decal face with a vertex more than `decalBoxClamp` metres outside the tier shell's
            # bounding box is dropped.
            m_ = float(CFG['decalBoxClamp'])
            pts_ = [o.matrix_world @ Vector(c) for o in shells for c in o.bound_box]
            lo_ = [min(q[k] for q in pts_) - m_ for k in range(3)]
            hi_ = [max(q[k] for q in pts_) + m_ for k in range(3)]
            mw_ = ob.matrix_world
            bad_ = [f_.index for f_ in ob.data.polygons
                    if any(not (lo_[k] <= (mw_ @ ob.data.vertices[v].co)[k] <= hi_[k]) for v in f_.vertices for k in range(3))]
            if bad_:
                bm_ = bmesh.new()
                bm_.from_mesh(ob.data)
                bm_.faces.ensure_lookup_table()
                bmesh.ops.delete(bm_, geom=[bm_.faces[i] for i in bad_], context='FACES')
                bmesh.ops.delete(bm_, geom=[v for v in bm_.verts if not v.link_faces], context='VERTS')
                bm_.to_mesh(ob.data)
                bm_.free()
                log(stage='decalBoxClamp', node=name, dropped=len(bad_))
        for f in ob.data.polygons:
            f.use_smooth = True
        if parent is not None and bpy.data.objects.get(parent) is not None:
            p_ob = bpy.data.objects[parent]
            ob.parent = p_ob
            ob.matrix_parent_inverse = p_ob.matrix_world.inverted()
        made[name] = triangles(ob)
    log(stage='decalsPlaced', made=made, budget=budget)
    return made


def soften_lamps(strength=2.0):
    """The lamp zones are driven by the runtime (LAMP_CHANNEL_BY_MATERIAL replaces the emissive);
    the GLB keeps only a modest authored glow so an offline viewer does not flare a whole patch."""
    for name in ('VehicleHeadlamp', 'VehicleTaillamp', 'VehicleIndicator', 'VehicleDRL', 'VehicleReverse'):
        mat = bpy.data.materials.get(name)
        if mat is None or not mat.use_nodes:
            continue
        for node in mat.node_tree.nodes:
            if node.type == 'BSDF_PRINCIPLED':
                es = node.inputs.get('Emission Strength')
                if es is not None and not es.is_linked:
                    es.default_value = min(es.default_value, strength)


def build_cage_tier(level_key, args, parts):
    """Replace the near shell, skins, glass and lamps with the cage-built shell for one tier."""
    spec = CFG[level_key]
    target = cage_mesh(args.cage, 3, '_limit_target')
    sources = zone_sources(target, parts)
    pieces = extract_decals(sources, target)
    strict = {m for p in pieces.values() for _, m in p['faces']}
    seam_on = spec.get('seams', level_key == 'cageMid' and bool(spec.get('articulated')))
    seams = extract_seams(_limit_tree(target)) if seam_on else None
    shell = cage_mesh(args.cage, spec['level'], 'CageShell')
    fit_to_limit(shell, target)
    # refine the zone edges on the CLOSED cage first, then cut. The refine re-fits every vertex to the
    # nearest limit-surface point; run after the booleans it pulled the cut walls (arch tunnels, inner arch caps,
    # the pickup's bed walls) out onto the skin, where they lay as inward-facing triangles over the flank.
    if spec.get('refineEdges'):
        refine_zone_edges(shell, sources, target, spec['refineEdges'], strict=strict)
    # `glassRefineExtra` {tier key: rounds} (coupe): extra refine rounds on body-owned glass outlines only (the fixed
    # quarter glass and screens; door glass left as is). One round drew the quarter glass lower edge as a stair step,
    # a second round everywhere ran over the far cap
    if (CFG.get('glassRefineExtra') or {}).get(level_key):
        refine_zone_edges(shell, sources, target, CFG['glassRefineExtra'][level_key], strict=strict,
                          only=lambda z: 'Glass' in z[0] and z[1][0] == 'body')
    cut_arches(shell, parts['arches'])
    if CFG.get('bedCut'):
        cut_bed(shell, parts['bed'])
    zones = project_zones(shell, sources, strict=strict)
    if spec.get('articulated'):
        zones = hold_moving_parts_inside(shell, zones, sources)
    assign_zones(shell, zones)
    lining = None
    lining_depth = spec.get('lining') or spec.get('glassLining')
    if lining_depth:
        # a coarse inner lining behind every non-glass face, so the greenhouse shows a dark cabin
        # instead of the sky through the back of the single-sided roof and doors
        lin = cage_mesh(args.cage, 0, 'CageLining')
        fit_to_limit(lin, target)
        bm = bmesh.new()
        bm.from_mesh(lin.data)
        bm.normal_update()
        for v in bm.verts:
            v.co -= v.normal * lining_depth
        bmesh.ops.reverse_faces(bm, faces=bm.faces)
        bm.to_mesh(lin.data)
        bm.free()
        cut_arches(lin, parts['arches'])
        if CFG.get('bedCut'):
            cut_bed(lin, parts['bed'])
            # no cabin behind the cab's back wall: the bed is lined by the near build's Bed_* statics, and a lining
            # there only folds through the bed walls when it is seated behind the coarse shell
            bm = bmesh.new()
            bm.from_mesh(lin.data)
            lmw0 = lin.matrix_world
            bmesh.ops.delete(bm, geom=[f for f in bm.faces if (lmw0 @ f.calc_center_median()).y > parts['bed']['y'][0] - 0.01], context='FACES')
            bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
            bm.to_mesh(lin.data)
            bm.free()
        # every lining vertex at least the lining depth behind the tier's own shell: the lining
        # faces into the car, and a double-sided lining material (the fleet's) would otherwise
        # show as a dark blot wherever the coarse lining crosses a concave stretch of the shell
        dgl = bpy.context.evaluated_depsgraph_get()
        stree = BVHTree.FromObject(shell, dgl)
        smw, smi = shell.matrix_world, shell.matrix_world.inverted()
        lmw, lmi = lin.matrix_world, lin.matrix_world.inverted()
        for v in lin.data.vertices:
            w = lmw @ v.co
            hit, nrm, _, _ = stree.find_nearest(smi @ w, 0.5)
            if hit is None:
                continue
            hit, nrm = smw @ hit, (smw.to_3x3() @ nrm).normalized()
            if (hit - w).dot(nrm) < 0.8 * lining_depth:
                v.co = lmi @ (hit - nrm * lining_depth)
        lin.data.update()
        # the lining runs behind the glass too: at mid range the greenhouse reads as tinted glass
        # over a dark cabin (verify-hero-greenhouse-integrity holds the see-through share to 10 %);
        # the cabin furniture is not drawn through it at this range
        lzones = project_zones(lin, sources, reach=0.12 + lining_depth)
        if not spec.get('lining'):
            # far tier: a lining only behind the glass, so the greenhouse reads as dark glass over
            # a cabin instead of the sky through the far side
            bm = bmesh.new()
            bm.from_mesh(lin.data)
            bm.faces.ensure_lookup_table()
            # a face is kept when ANY of its corners lies behind glass, so the coarse lining
            # covers the whole greenhouse instead of only the faces mostly under it
            lmw_ = lin.matrix_world
            behind = []
            for poly, (m, _) in zip(lin.data.polygons, lzones):
                pts = [lmw_ @ lin.data.vertices[v].co for v in poly.vertices]
                glass_ = glass_names()   # `tintAsGlass`: the tinted rear glass is glass too
                behind.append(m in glass_ or any(
                    _zone_at(pt, sources, 0.12 + lining_depth)[0] in glass_ for pt in pts))
            bmesh.ops.delete(bm, geom=[bm.faces[i] for i, keep in enumerate(behind) if not keep], context='FACES')
            bm.to_mesh(lin.data)
            bm.free()
            lzones = [z for z, keep in zip(lzones, behind) if keep]
        # a coarse lining face rides on a moving panel only if every corner lies over that panel;
        # one that straddles a shut line stays on the body instead of lifting a piece of wing
        mw = lin.matrix_world
        owners = []
        for poly, (_, o) in zip(lin.data.polygons, lzones):
            corners = {_zone_at(mw @ lin.data.vertices[v].co, sources, 0.12 + lining_depth)[1]
                       for v in poly.vertices}
            owners.append(o if corners == {o} else ('body', None))
        lining = [('VehicleInterior', o) for o in owners]
        assign_zones(lin, lining)
    bpy.data.objects.remove(target, do_unlink=True)
    # the near shell, skins, glass, lamps, grilles and recess cups give way to the cage shell
    gone = drop(lambda o: o.name in SHELL or o.name.startswith(SHELL_REPLACES + tuple(CFG.get('shellReplacesExtra', ())))
                or o.name.endswith('_Seal'))
    made = split_owners(shell, zones) if spec.get('articulated') else {'Body': triangles(shell)}
    if not spec.get('articulated'):
        shell.name = 'Body'
        shell.data.name = 'Body'
    if lining is not None:
        if spec.get('articulated'):
            made.update(split_owners(lin, lining, suffix='_Lining'))
        else:
            lin.name = lin.data.name = 'Body_Lining'
            made['Body_Lining'] = triangles(lin)
    shells = [o for o in meshes() if o.name in made and not o.name.endswith('_Lining')]
    decals = place_decals(pieces, shells, spec.get('decalOffset', 0.004), spec.get('decalBudget', 1500),
                          bool(spec.get('articulated')))
    soften_lamps()
    tips = tips_to_shell(shells, CFG.get('tipsToShell', ()))
    trims = rebuild_trims(parts, shells, spec.get('trimPrefixes', ('ArchMould', 'ArchLip')),
                          spec.get('trimSides', 5), spec.get('trimStep', 0.06))
    seam_made = build_seams(seams, shells, spec.get('seamWidth', SEAM_WIDTH), spec.get('seamOffset', SEAM_OFFSET)) if seams else None
    guard = clear_arch_faces(parts['arches'])
    out = dict(level=spec['level'], shell=made, dropped=len(gone), trims=trims, decals=decals, archGuard=guard,
               seams=seam_made)
    if CFG.get('tipsToShell'):
        out['tips'] = tips
    return out


def tips_to_shell(shells, prefixes, proud=0.012):
    """`tipsToShell`: exhaust tips modelled at the near build's recessed tail sit behind the recess-less cage shell.
    Move each tip object (and its _Inner) back along +Y until its rear end stands `proud` out of the tier's shell,
    measured by a ray from behind the car at the tip's centre."""
    if not prefixes:
        return {}
    dg = bpy.context.evaluated_depsgraph_get()
    trees = [(o.matrix_world, o.matrix_world.inverted(), BVHTree.FromObject(o, dg)) for o in shells]
    out = {}
    for ob in [o for o in meshes() if o.name.startswith(tuple(prefixes)) and not o.name.endswith('_Inner')]:
        mw = ob.matrix_world
        pts = [mw @ v.co for v in ob.data.vertices]
        c = sum(pts, Vector()) / len(pts)
        rear = max(p.y for p in pts)
        hit_y = None
        for smw, smi, tree in trees:
            o_, d_ = smi @ Vector((c.x, 4.0, c.z)), (smi.to_3x3() @ Vector((0, -1, 0))).normalized()
            loc = tree.ray_cast(o_, d_, 6.0)[0]
            if loc is not None:
                y = (smw @ loc).y
                hit_y = y if hit_y is None else max(hit_y, y)
        if hit_y is None or rear >= hit_y + proud:
            continue
        dy = hit_y + proud - rear
        for o in (ob, bpy.data.objects.get(ob.name + '_Inner')):
            if o is not None:
                o.location.y += dy
        out[ob.name] = round(dy, 4)
    bpy.context.view_layer.update()
    log(stage='tipsToShell', moved=out)
    return out


# --------------------------------------------------------------------------- shut-line seams

SEAM_PANELS = ('HoodSkin', 'TrunkSkin', 'Panel_FL', 'Panel_FR', 'Panel_RL', 'Panel_RR', 'Panel_SlideR', 'Panel_SlideL')
SEAM_WIDTH = 0.010     # m: the dark strip drawn along a panel outline at the mid tier
SEAM_OFFSET = 0.003    # m: out from the tier's shell (under the 4 mm decals)


def _simplify(pts, tol):
    """Douglas-Peucker on an open 3D polyline."""
    if len(pts) < 3:
        return list(pts)
    a, b = pts[0], pts[-1]
    ab = b - a
    L2 = ab.length_squared
    best, bi = -1.0, 0
    for i in range(1, len(pts) - 1):
        ap = pts[i] - a
        d = (ap - ab * (ap.dot(ab) / L2)).length if L2 > 1e-12 else ap.length
        if d > best:
            best, bi = d, i
    if best <= tol:
        return [a, b]
    return _simplify(pts[:bi + 1], tol)[:-1] + _simplify(pts[bi:], tol)


def extract_seams(lim, min_len=0.25, min_loop=0.9, tol=0.003, max_seg=0.08):
    """The near build's shut lines: on each moving panel skin, the edges where its OUTER paint surface (faces on the
    limit surface, facing out) ends. Chained, simplified to `tol` and resampled to `max_seg`, in world space.
    Returns [(owner, points, closed)]."""
    chains = []
    for name in SEAM_PANELS:
        ob = bpy.data.objects.get(name)
        if ob is None or ob.type != 'MESH':
            continue
        mw = ob.matrix_world
        n3 = mw.to_3x3().inverted().transposed()
        slots = [sl.material.name if sl.material else 'VehiclePaint' for sl in ob.material_slots]
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        outer = set()
        for f in bm.faces:
            mat = slots[min(f.material_index, len(slots) - 1)] if slots else 'VehiclePaint'
            if mat == 'VehicleInterior' or mat in glass_names():   # `tintAsGlass`: tinted glass is glass too
                continue
            at = _limit_at(lim, mw @ f.calc_center_median(), 0.008)
            if at is not None and (n3 @ f.normal).normalized().dot(at[1]) > 0.7:
                outer.add(f.index)
        adj = {}
        turn = {}
        for e in bm.edges:
            if sum(1 for g in e.link_faces if g.index in outer) != 1:
                continue
            a, b = e.verts[0].index, e.verts[1].index
            # [exoticCoupe run] a shut line is where the paint turns ~90 deg into the gap wall; an interior paint
            # boundary (a deep crease's faces off the limit surface) meets a near-parallel neighbour
            fo = next(g for g in e.link_faces if g.index in outer)
            others = [g for g in e.link_faces if g.index not in outer]
            turn[(a, b)] = turn[(b, a)] = (not others) or any(abs(fo.normal.dot(g.normal)) < 0.5 for g in others)
            adj.setdefault(a, []).append(b)
            adj.setdefault(b, []).append(a)
        co = {v.index: mw @ v.co for v in bm.verts}
        bm.free()
        seen = set()
        starts = [v for v, ns in adj.items() if len(ns) != 2] + list(adj)
        for s0 in starts:
            for nb in list(adj[s0]):
                if (s0, nb) in seen:
                    continue
                path = [s0]
                prev, cur = s0, nb
                seen.add((s0, nb))
                seen.add((nb, s0))
                while True:
                    path.append(cur)
                    nxt = [w for w in adj[cur] if (cur, w) not in seen]
                    if len(adj[cur]) != 2 or not nxt:
                        break
                    w = nxt[0]
                    seen.add((cur, w))
                    seen.add((w, cur))
                    prev, cur = cur, w
                closed = path[0] == path[-1]
                pts = [co[i] for i in path]
                length = sum((b - a).length for a, b in zip(pts, pts[1:]))
                # [exoticCoupe run] `seamOutlineOnly`: keep a chain only where most of its edges turn into a gap wall
                if CFG.get('seamOutlineOnly'):
                    k_ = sum(1 for a_, b_ in zip(path, path[1:]) if turn.get((a_, b_)))
                    if k_ < 0.35 * max(1, len(path) - 1):     # the door outline measured 51 %, the strays 0-33 %
                        log(stage='seamDropped', panel=name, length=round(length, 3), turning=k_, edges=len(path) - 1,
                            start=[round(v, 3) for v in pts[0]], end=[round(v, 3) for v in pts[-1]])
                        continue
                # small closed loops are recess and lamp cut-outs (plate, handle, tail lamps), not shut lines
                if length < (min_loop if closed else min_len):
                    continue
                pts = _simplify(pts, tol)
                dense = [pts[0]]
                for a, b in zip(pts, pts[1:]):
                    k = max(1, int(math.ceil((b - a).length / max_seg)))
                    dense += [a.lerp(b, j / k) for j in range(1, k + 1)]
                chains.append((_owner_of(ob), dense, closed))
    log(stage='seamsFound', chains=len(chains), points=sum(len(c[1]) for c in chains))
    return chains


def build_seams(chains, shells, width, offset):
    """Lay each shut line onto the tier's own shells as a `width` ribbon of VehicleTrimBlack, `offset` out along the
    shell normal, as a separate `<panel>_Seam` object riding on its moving part (joined into the panel's *_Trim
    node after the budget, see finish_seams)."""
    dg = bpy.context.evaluated_depsgraph_get()
    trees = [(o.matrix_world, o.matrix_world.inverted(), BVHTree.FromObject(o, dg)) for o in shells]

    def onto(co):
        best = None
        for mw, mi, tree in trees:
            hit, nrm, _, dist = tree.find_nearest(mi @ co, 0.05)
            if hit is not None and (best is None or dist < best[0]):
                best = (dist, mw @ hit, (mw.to_3x3() @ nrm).normalized())
        return best

    mat = bpy.data.materials.get('VehicleTrimBlack') or bpy.data.materials.new('VehicleTrimBlack')
    groups = {}
    for owner, pts, closed in chains:
        groups.setdefault(_decal_owner_name(owner), []).append((pts, closed))
    made = {}
    for (trim_name, parent), plist in groups.items():
        name = trim_name.replace('_Trim', '_Seam')
        bm = bmesh.new()
        for pts, closed in plist:
            proj = [onto(p) for p in pts]
            rows = []
            for i, pr in enumerate(proj):
                if pr is None:
                    rows.append(None)
                    continue
                j0, j1 = max(0, i - 1), min(len(pts) - 1, i + 1)
                t = pts[j1] - pts[j0]
                if t.length < 1e-9:
                    rows.append(None)
                    continue
                n = pr[2]
                side = n.cross(t.normalized())
                if side.length < 1e-6:
                    rows.append(None)
                    continue
                side.normalize()
                base = pr[1] + n * offset
                rows.append((bm.verts.new(base + side * width * 0.5), bm.verts.new(base - side * width * 0.5), n))
            pairs = list(zip(rows, rows[1:])) + ([(rows[-1], rows[0])] if closed and len(rows) > 2 else [])
            for ra, rb in pairs:
                if ra is None or rb is None:
                    continue
                try:
                    f = bm.faces.new((ra[0], ra[1], rb[1], rb[0]))
                except ValueError:
                    continue
                f.normal_update()
                if f.normal.dot(ra[2]) < 0:
                    f.normal_flip()
        bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
        me = bpy.data.meshes.new(name)
        bm.to_mesh(me)
        bm.free()
        me.materials.append(mat)
        ob = bpy.data.objects.new(name, me)
        bpy.context.scene.collection.objects.link(ob)
        ob['_seam'] = True
        if parent is not None and bpy.data.objects.get(parent) is not None:
            p_ob = bpy.data.objects[parent]
            ob.parent = p_ob
            ob.matrix_parent_inverse = p_ob.matrix_world.inverted()
        made[name] = triangles(ob)
    log(stage='seams', made=made)
    return made


def finish_seams(offset=SEAM_OFFSET):
    """After the budget merge (which re-triangulates the shells): push each seam vertex back to `offset` out from the
    final shell, then join each <panel>_Seam into its <panel>_Trim node (created if the panel has no decals)."""
    dg = bpy.context.evaluated_depsgraph_get()
    shells = [o for o in meshes() if (o.name in ('Body', 'HoodSkin', 'TrunkSkin') or o.name.startswith(('Panel_', 'Window_')))
              and not o.name.endswith(('_Lining', '_Trim', '_Seam'))]
    trees = [(o.matrix_world, o.matrix_world.inverted(), BVHTree.FromObject(o, dg)) for o in shells]
    out = {}
    for seam in [o for o in meshes() if o.get('_seam')]:
        mw, mi = seam.matrix_world, seam.matrix_world.inverted()
        for v in seam.data.vertices:
            w = mw @ v.co
            best = None
            for smw, smi, tree in trees:
                hit, nrm, _, dist = tree.find_nearest(smi @ w, 0.05)
                if hit is not None and (best is None or dist < best[0]):
                    best = (dist, smw @ hit, (smw.to_3x3() @ nrm).normalized())
            if best is not None:
                n = best[2] if best[2].dot(w - best[1]) >= 0 else -best[2]
                v.co = mi @ (best[1] + n * offset)
        seam.data.update()
        trim_name = seam.name.replace('_Seam', '_Trim')
        trim = bpy.data.objects.get(trim_name)
        del seam['_seam']
        if trim is None:
            seam.name = seam.data.name = trim_name
            out[trim_name] = triangles(seam)
            continue
        bpy.ops.object.select_all(action='DESELECT')
        trim.select_set(True)
        seam.select_set(True)
        bpy.context.view_layer.objects.active = trim
        bpy.ops.object.join()
        out[trim_name] = triangles(trim)
    return out


def seat_liners(wall=0.006):
    """Keep the near build's wheel-well liners (Arch_*, VehicleUnderbody) behind the tier's flank. The
    near well is opened wider than the tier's arch cylinder (lip returns, widened lower corners), and the table's
    60-triangle collapse moves the liner rim further, so the reduced liner poked out through the coarse flank as
    dark triangles just ahead of and behind each wheel. Every liner vertex outboard of the shell's inner face at its
    own station and height (ray cast along -X, as seat_arch_lips does on the near tier) is pressed onto it."""
    dg = bpy.context.evaluated_depsgraph_get()
    shells = [o for o in meshes() if (o.name in ('Body', 'HoodSkin', 'TrunkSkin') or o.name.startswith('Panel_'))
              and not o.name.endswith(('_Lining', '_Trim', '_Seam'))]
    trees = [(o, BVHTree.FromObject(o, dg), o.matrix_world.inverted()) for o in shells]
    out = {}
    for ob in meshes():
        if not ob.name.startswith(('Arch_F_', 'Arch_R_')):
            continue
        mw, mi = ob.matrix_world, ob.matrix_world.inverted()
        moved = 0
        for v in ob.data.vertices:
            w = mw @ v.co
            side = 1.0 if w.x > 0 else -1.0
            best = None
            for o, tree, inv in trees:
                hit = tree.ray_cast(inv @ Vector((side * 2.0, w.y, w.z)), (inv.to_3x3() @ Vector((-side, 0.0, 0.0))).normalized(), 3.0)[0]
                if hit is not None:
                    hx = side * (o.matrix_world @ hit).x
                    if hx > 0:
                        best = hx if best is None else max(best, hx)
            if best is not None and abs(w.x) > best - wall:
                w.x = side * (best - wall)
                v.co = mi @ w
                moved += 1
        ob.data.update()
        out[ob.name] = moved
    return out


def limit_normals(cage_path, sharp_deg=50.0, reach=0.008):
    """Shade every cage-tier shell piece from the limit surface it was fitted to: the budget's planar merge
    leaves long sliver triangles whose averaged vertex normals shade a gently curved flank as dark triangular
    patches. Edges sharper than `sharp_deg` (arch lips, bed rails, cut walls) are split first; each corner of a face
    that lies on the skin and faces with it takes the limit surface's interpolated normal at that vertex."""
    target = cage_mesh(cage_path, 3, '_limit_normals')
    lim = _limit_tree(target)
    bpy.data.objects.remove(target, do_unlink=True)
    out = {}
    for ob in meshes():
        n = ob.name
        if not (n in ('Body', 'HoodSkin', 'TrunkSkin') or n.startswith(('Panel_', 'Window_'))) or n.endswith(('_Lining', '_Trim', '_Seam')):
            continue
        me = ob.data
        bm = bmesh.new()
        bm.from_mesh(me)
        for e in bm.edges:
            if len(e.link_faces) == 2 and e.calc_face_angle(0.0) > math.radians(sharp_deg):
                e.smooth = False
        bm.to_mesh(me)
        bm.free()
        for f in me.polygons:
            f.use_smooth = True
        me.update()
        mw = ob.matrix_world
        n3 = mw.to_3x3().inverted().transposed()
        back = mw.to_3x3().transposed()
        cur = me.corner_normals
        normals = [Vector(cur[i].vector) for i in range(len(me.loops))]
        set_ = 0
        cache = {}
        for poly in me.polygons:
            fn = (n3 @ poly.normal).normalized()
            for li in poly.loop_indices:
                vi = me.loops[li].vertex_index
                if vi not in cache:
                    cache[vi] = _limit_at(lim, mw @ me.vertices[vi].co, reach)
                at = cache[vi]
                if at is None or at[1].dot(fn) < 0.3:
                    continue
                normals[li] = (back @ at[1]).normalized()
                set_ += 1
        me.normals_split_custom_set(normals)
        out[n] = set_
    return out


def reseat_plates(offset=None, most=0.008):
    """A cage tier has no plate recess: an anchor seated in the near build's recess ends up
    inside the coarse shell (or, where a mount is dropped, floating ahead of it). Each plate is set `offset` out from
    the outermost surface of the tier's own meshes straight behind its centre, and further out (up to `most`) only if
    the surface 12 cm either side stands proud of the centre, so the 3-9 mm anchor contract holds on every tier.
    it may now move back as well as forward; only the anchors themselves are excluded from the targets.
    `offset` defaults to the body's `plateProudM` (3 mm when it has none)."""
    if offset is None:
        offset = CFG.get('plateProudM', 0.003)
    dg = bpy.context.evaluated_depsgraph_get()
    others = [o for o in meshes() if o.name not in ('PlateFront', 'PlateRear') and not o.name.endswith('_Lining')]
    trees = [(o, BVHTree.FromObject(o, dg), o.matrix_world.inverted()) for o in others]
    out = {}
    for name, _, face in CFG['plates']:
        p = bpy.data.objects.get(name)
        if p is None:
            continue
        c = p.matrix_world.translation.copy()
        best = {}
        # the ray starts ahead of the body's end: 4 m sat inside the 12.5 m bus (unchanged for bodies up to 6 m)
        R0 = max(4.0, CFG['lengthM'] / 2 + 1.0)
        for o, tree, inv in trees:
            for dx in (-0.12, 0.0, 0.12):
                hit = tree.ray_cast(inv @ Vector((c.x + dx, face * R0, c.z)), (inv.to_3x3() @ Vector((0.0, -face, 0.0))).normalized(), 2 * R0)[0]
                if hit is not None:
                    w = o.matrix_world @ hit
                    if dx not in best or face * w.y > face * best[dx]:
                        best[dx] = w.y
        if 0.0 not in best:
            out[name] = None
            continue
        centre = best[0.0]
        edge = max((face * best[d] for d in (-0.12, 0.12) if d in best), default=face * centre)
        proud = min(most, max(offset, edge - face * centre + offset))
        moved = centre + face * proud - c.y
        p.location.y += moved
        out[name] = round(moved * 1000, 1)
    log(stage='reseatPlates', movedMm=out)
    return out


def fix_plate_anchor_names():
    """A build that carries its own `PlateFront`/`PlateRear` mesh (the fullSizeSuv's blank, origin at the
    world origin) pushed the runtime anchor to `PlateFront.001`, so the runtime bound the blank. At the cage tiers the
    blank gives way to the card (as the minivan's `plateBlanks` do) and the anchor takes the runtime name."""
    out = {}
    for name in ('PlateFront', 'PlateRear'):
        anchor = bpy.data.objects.get(name + '.001')
        blank = bpy.data.objects.get(name)
        if anchor is None or blank is None:
            continue
        bpy.data.objects.remove(blank, do_unlink=True)
        anchor.name = name
        if anchor.data is not None and anchor.data.users == 1:
            anchor.data.name = name
        out[name] = anchor.name
    return out


def cut_bed(ob, bed):
    """Open the pickup's cargo bed in a cage-tier shell with the build's own bed box (bed.py `cut`:
    |x| < wallX, y0 < y < y1, above floorZ + 0.1); the near tier's Bed_* statics line it."""
    y0, y1 = bed['y']
    wx = bed['wallX']
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    for v in bm.verts:
        v.co = Vector((v.co.x * 2 * wx, (y0 + y1) / 2 + v.co.y * (y1 - y0), (bed['floorZ'] + 0.1 + 2.5) / 2 + v.co.z * (2.5 - bed['floorZ'] - 0.1)))
    me = bpy.data.meshes.new('_bed_cut')
    bm.to_mesh(me)
    bm.free()
    cut = bpy.data.objects.new('_bed_cut', me)
    bpy.context.scene.collection.objects.link(cut)
    md = ob.modifiers.new('bed', 'BOOLEAN')
    md.operation = 'DIFFERENCE'
    md.object = cut
    md.solver = 'EXACT'
    bpy.context.view_layer.objects.active = ob
    bpy.ops.object.modifier_apply(modifier=md.name)
    bpy.data.objects.remove(cut, do_unlink=True)
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if len(f.verts) > 4])
    bm.to_mesh(ob.data)
    bm.free()
    for f in ob.data.polygons:
        f.use_smooth = True


def _cfg_rule(key, name):
    """(target, protect) from a BODIES entry's per-prefix table, or None when no prefix matches."""
    for prefix, rule in (CFG.get(key) or {}).items():
        if name.startswith(prefix):
            return rule
    return None


def clear_arch_faces(arches, margin=0.012):
    """Geometric guard: delete every shell / skin / lining face whose centre lies inside an arch
    cylinder (parts.json arches, less `margin`). On this body the mid tier came out with the rear arches skinned over
    (126 paint faces over each rear wheel) although the boolean cut itself opens them; whatever re-covers them, a face
    inside the wheel's cylinder can never be body."""
    out = {}
    for ob in list(meshes()):
        n = ob.name
        if not (n in ('Body', 'HoodSkin', 'TrunkSkin') or n.startswith(('Panel_', 'Body_')) or n.endswith(('_Lining', '_Trim'))):
            continue
        mw = ob.matrix_world
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        kill = []
        for f in bm.faces:
            c = mw @ f.calc_center_median()
            for a in arches:
                if a['x'][0] + margin < abs(c.x) and (c.y - a['y']) ** 2 + (c.z - a['z']) ** 2 < (a['r'] - margin) ** 2:
                    kill.append(f)
                    break
        if kill:
            bmesh.ops.delete(bm, geom=kill, context='FACES')
            bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
            bm.to_mesh(ob.data)
            out[n] = len(kill)
        bm.free()
    log(stage='archGuard', removed=out)
    return out


def cage_mid_table(ob):
    n = ob.name
    if n.endswith('_Seam'):
        return None, False   # shut-line ribbons: a few hundred triangles, never reduced
    rule = _cfg_rule('midTargets', n)
    if rule is not None:
        return tuple(rule)
    if n in ('Body', 'HoodSkin', 'TrunkSkin') or n.startswith(('Panel_', 'Window_')) or n.endswith(('_Lining', '_Trim')):
        return None, False
    if n.startswith(('ArchMould', 'ArchLip')):
        return None, False
    return mid_table(ob)


def cage_far_table(ob):
    n = ob.name
    if n.endswith('_Seam'):
        return None, False
    rule = _cfg_rule('farTargets', n)
    if rule is not None:
        return tuple(rule)
    if n in ('Body', 'Body_Lining', 'Body_Trim') or n.startswith(('ArchMould', 'ArchLip')):
        return None, False
    return far_table(ob)


def cage_far_keep(ob):
    n = ob.name
    if n in SIGN_NODES:
        return True
    if n.startswith(tuple(CFG.get('farKeep', ()))):
        return True
    if n in ('Body', 'Body_Lining', 'Body_Trim', 'PlateFront', 'PlateRear') or n.startswith(('ArchMould', 'ArchLip', 'Arch_F_', 'Arch_R_')):
        return True
    if n.startswith('Wheel_') and n.split('_', 2)[-1] in ('Tyre', 'Lip'):
        return True
    # roof rails: a few faces, but they are the top of the silhouette and of the bounding box
    return n.startswith(('Mirror_L_Housing', 'Mirror_R_Housing', 'RoofRail_'))


def seat_cage_linings(depth, margin=0.004):
    """After the budget fit (whose planar merge can sink a domed bonnet onto its lining), push
    every *_Lining vertex, and every lining face's centre, back behind the tier's final shells."""
    dg = bpy.context.evaluated_depsgraph_get()
    shells = [o for o in meshes() if o.name in ('Body', 'HoodSkin', 'TrunkSkin') or
              (o.name.startswith(('Panel_', 'Window_')) and not o.name.endswith(('_Lining', '_Trim')))]
    trees = [(o.matrix_world, o.matrix_world.inverted(), BVHTree.FromObject(o, dg)) for o in shells]

    def nearest(w):
        best = None
        for mw, mi, tree in trees:
            hit, nrm, _, dist = tree.find_nearest(mi @ w, 0.5)
            if hit is not None and (best is None or dist < best[0]):
                best = (dist, mw @ hit, (mw.to_3x3() @ nrm).normalized())
        return best

    moved = 0
    for lin in [o for o in meshes() if o.name.endswith('_Lining')]:
        mw, mi = lin.matrix_world, lin.matrix_world.inverted()
        me = lin.data
        for _ in range(4):
            sunk = 0
            for p in me.polygons:
                cos = [mw @ me.vertices[i].co for i in p.vertices]
                pts = cos + [sum(cos, Vector()) / len(cos)]
                worst, wn = 0.0, None
                for pt in pts:
                    b = nearest(pt)
                    if b is None:
                        continue
                    deficit = margin - (b[1] - pt).dot(b[2])
                    if worst < deficit < 0.08:
                        worst, wn = deficit, b[2]
                if wn is None:
                    continue
                for i in p.vertices:
                    me.vertices[i].co = mi @ ((mw @ me.vertices[i].co) - wn * (worst + 0.25 * depth))
                sunk += 1
            moved += sunk
            if not sunk:
                break
        me.update()
    return moved


def reseat_decals(offset, most=0.04):
    """[exoticCoupe run] The decals are laid `offset` out from the shell BEFORE the budget fit; the shell's planar merge
    then moved chords outward across concave stretches (the tail band) and hid 14-35 % of the rear decal triangles up to
    ~17 mm deep. Push every *_Trim vertex (decals and seam ribbons) back out to `offset` in front of the final shells
    (vertices more than `most` behind are left alone: not on this surface)."""
    dg = bpy.context.evaluated_depsgraph_get()
    shells = [o for o in meshes() if o.name in ('Body', 'HoodSkin', 'TrunkSkin') or
              (o.name.startswith('Panel_') and not o.name.endswith(('_Lining', '_Trim')))]
    trees = [(o.matrix_world, o.matrix_world.inverted(), BVHTree.FromObject(o, dg)) for o in shells]
    moved = {}
    for ob in [o for o in meshes() if o.name.endswith('_Trim')]:
        mw, mi = ob.matrix_world, ob.matrix_world.inverted()
        n_ = 0
        for v in ob.data.vertices:
            w = mw @ v.co
            best = None
            for smw, smi, tree in trees:
                hit, nrm, _, dist = tree.find_nearest(smi @ w, 0.06)
                if hit is not None and (best is None or dist < best[0]):
                    best = (dist, smw @ hit, (smw.to_3x3() @ nrm).normalized())
            if best is None:
                continue
            h = (w - best[1]).dot(best[2])
            if -most < h < offset:
                v.co = mi @ (w + best[2] * (offset - h))
                n_ += 1
        ob.data.update()
        if n_:
            moved[ob.name] = n_
    log(stage='decalsSeated', moved=moved)
    return moved


def fit_cage_budget(cap, table):
    """Parts to their table targets; if the tier is still over, merge the shell's flattest faces."""
    reductions = []
    for ob in list(meshes()):
        target, protect = table(ob)
        if target is not None and triangles(ob) > target:
            reductions.append(reduce_protected(ob, target, protect))
    planar = None
    for angle in (0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0):
        if total_triangles() <= cap:
            break
        planar = planar_skins(angle, with_body=True)
    for ob in bpy.context.scene.objects:
        if '_planarDone' in ob:
            del ob['_planarDone']
    if total_triangles() > cap:
        census = sorted(((o.name, triangles(o)) for o in meshes()), key=lambda r: -r[1])
        log(stage='overCap', objects=len(census), census=census[:60], smallTotal=sum(t for _, t in census if t <= 48))
        raise ValueError(f'cage tier over its cap: {total_triangles()} > {cap}')
    return dict(reductions=reductions, shellPlanar=planar)


def paint_as():
    """`paintAs` bodies: VehiclePaint leaves under an untinted livery name with its own colour.

    The runtime tints VehiclePaint per vehicle from the catalogue paint set; a livery body (the taxi yellow) must
    keep its authored colour, so it ships under the name the fleet's livery bodies already use.
    """
    spec = CFG.get('paintAs')
    mat = bpy.data.materials.get('VehiclePaint') if spec else None
    if mat is None:
        return None
    mat.name = spec['name']
    bsdf = next((n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None) if mat.use_nodes else None
    if bsdf is not None:
        for socket, value in (('Base Color', spec['baseColor']), ('Metallic', spec['metallic']),
                              ('Roughness', spec['roughness'])):
            for link in list(bsdf.inputs[socket].links):
                mat.node_tree.links.remove(link)
            bsdf.inputs[socket].default_value = value
    mat.diffuse_color = spec['baseColor']
    return mat.name


def paint_rename(path):
    """`paintRename` bodies: VehiclePaint leaves under an untinted livery name with its authored colour.

    Unlike `paintAs` (which sets a colour), only the material's name changes, in the GLB's JSON chunk after export:
    the patrol sedan's black body keeps the reviewed paint values and the runtime, which tints only VehiclePaint,
    leaves it black.
    """
    new = CFG.get('paintRename')
    if not new:
        return None
    raw = Path(path).read_bytes()
    n = struct.unpack('<I', raw[12:16])[0]
    doc = json.loads(raw[20:20 + n])
    rest = raw[20 + n:]
    names = {m.get('name') for m in doc.get('materials', [])}
    if new in names:
        raise ValueError(f'{new} already present in {Path(path).name}')
    hit = [m for m in doc.get('materials', []) if m.get('name') == 'VehiclePaint']
    for m in hit:
        m['name'] = new
    js = json.dumps(doc, separators=(',', ':')).encode()
    js += b' ' * ((4 - len(js) % 4) % 4)
    Path(path).write_bytes(struct.pack('<III', 0x46546C67, 2, 12 + 8 + len(js) + len(rest))
                           + struct.pack('<II', len(js), 0x4E4F534A) + js + rest)
    return len(hit)


LAMP_ZONE_MATERIALS = {
    # the fleet's indicator (as the other skill bodies ship it) and a stop lamp with the tail lamp's own values
    'VehicleIndicator': dict(baseColor=(0.9, 0.4, 0.0, 1.0), emission=(1.0, 0.45, 0.0, 1.0), strength=6.0),
    'VehicleBrakeLamp': dict(),
}


def lamp_zones(rules=None):
    """`lampZones` bodies: faces of a lamp material inside a box move to another lamp material, nothing else.

    A build with no indicator or stop-lamp faces of its own (the mid-engine coupe: one red strip per tail lamp, no
    amber anywhere) gets those channels by re-assigning faces it already has, on every tier, just before export.
    Each rule is (target, source materials, |x| range, y range, z range) in the Blender frame (nose to -Y), tested on
    the face centroid. No vertex moves: only `material_index` changes. VehicleBrakeLamp copies VehicleTaillamp as the
    tier left it; VehicleIndicator copies it with the fleet's amber (emission strength capped by the tier's tail lamp).
    """
    rules = rules or CFG.get('lampZones')
    if not rules:
        return None
    tail = bpy.data.materials.get('VehicleTaillamp')
    if tail is None:
        raise ValueError('lampZones needs a VehicleTaillamp material to copy')
    made = {}
    for target in {r[0] for r in rules}:
        mat = bpy.data.materials.get(target)
        if mat is None:
            mat = tail.copy()
            mat.name = target
            spec = LAMP_ZONE_MATERIALS[target]
            bsdf = next((n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None) if mat.use_nodes else None
            if bsdf is not None and spec:
                for socket, value in (('Base Color', spec['baseColor']), ('Emission Color', spec['emission'])):
                    if bsdf.inputs.get(socket) is not None:
                        for link in list(bsdf.inputs[socket].links):
                            mat.node_tree.links.remove(link)
                        bsdf.inputs[socket].default_value = value
                es = bsdf.inputs.get('Emission Strength')
                if es is not None and not es.is_linked:
                    es.default_value = min(es.default_value, spec['strength'])
                mat.diffuse_color = spec['baseColor']
        made[target] = 0
    for ob in meshes():
        slots = [s.material.name if s.material else '' for s in ob.material_slots]
        if not any(n in r[1] for r in rules for n in slots):
            continue
        mw = ob.matrix_world
        me = ob.data
        index_of = {}
        # a face index past the last slot draws with the last slot (Blender clamps it); pin it there before any slot
        # is appended, or the new slot would capture it
        last = len(slots) - 1
        for pl in me.polygons:
            if pl.material_index > last:
                pl.material_index = last
        for pl in me.polygons:
            name = slots[pl.material_index]
            c = mw @ pl.center
            for target, sources, (x0, x1), (y0, y1), (z0, z1) in rules:
                if name in sources and x0 <= abs(c.x) < x1 and y0 < c.y < y1 and z0 < c.z < z1:
                    if target not in index_of:
                        if target not in slots:
                            me.materials.append(bpy.data.materials[target])
                            slots.append(target)
                        index_of[target] = slots.index(target)
                    pl.material_index = index_of[target]
                    made[target] += 1
                    break
    log(stage='lampZones', faces=made)
    return made


def tier_glass(level):
    """`tierGlass` bodies: darken VehicleGlass / VehicleGlassTint for a mid or far export (level 1 or 2). LOD0 is
    never touched: each cage tier reopens the saved near/reduce blend, so the change does not reach the near export."""
    if not level or not CFG.get('tierGlass'):
        return None
    spec = {**TIER_GLASS, **(CFG['tierGlass'] if isinstance(CFG['tierGlass'], dict) else {})}
    out = {}
    for name, v in spec.items():
        mat = bpy.data.materials.get(name)
        if mat is None:
            continue
        bsdf = next((n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None) if mat.use_nodes else None
        if bsdf is None:
            continue
        for socket, value in (('Base Color', v['baseColor']), ('Alpha', v['alpha'])):
            inp = bsdf.inputs.get(socket)
            if inp is None:
                continue
            for link in list(inp.links):
                mat.node_tree.links.remove(link)
            inp.default_value = value
        mat.diffuse_color = (*v['baseColor'][:3], v['alpha'])
        mat.surface_render_method = 'BLENDED'
        out[name] = dict(baseColor=list(v['baseColor'][:3]), alpha=v['alpha'])
    log(stage='tierGlass', lod=level, glass=out)
    return out


ARTICULATED_ROOTS = ('Door_FL', 'Door_FR', 'Door_RL', 'Door_RR', 'Hood', 'Trunk', 'SlideDoor_R', 'SlideDoor_L')


def export_fold(level):
    """`exportFold` bodies: material folds on the articulated parts of one tier only, applied just before that tier is
    written (after every budget, zone and seam pass), so no other tier and no vertex changes.

    The runtime draws each door and lid as its own pools (one draw per material per part, plus one per material of
    the part's window) and every part material once more in the shut batch, against fixed budgets (8 shut, 32 pool
    draws; verify:vehicle-articulation). The retrofit put two new materials on the moving parts: the tinted rear glass
    (VehicleGlassTint, which the overlay draws as its own material) on the tailgates and rear doors, and the frit
    ribbon (trim black) on the door glass, one window draw per door. Each rule is (levels, scope, {source: target}):
    scope 'parts' is every mesh under a door, lid or sliding door; 'windows' only the meshes under `Window_<door>`.
    """
    rules = [r for r in (CFG.get('exportFold') or ()) if level in r[0]]
    if not rules:
        return None
    out = {}
    for levels, scope, fold in rules:
        roots = [bpy.data.objects.get(n) for n in ARTICULATED_ROOTS]
        if scope == 'windows':
            roots = [bpy.data.objects.get(f'Window_{n[-2:]}') for n in ARTICULATED_ROOTS if n.startswith(('Door_', 'SlideDoor_'))]
        obs = {o for r in roots if r is not None for o in [r] + list(r.children_recursive) if o.type == 'MESH'}
        for ob in sorted(obs, key=lambda o: o.name):
            for k, slot in enumerate(ob.material_slots):
                dst = fold.get(slot.material.name if slot.material else '')
                if not dst:
                    continue
                if slot.link == 'OBJECT':
                    slot.material = bpy.data.materials[dst]
                else:
                    if ob.data.users > 1:
                        ob.data = ob.data.copy()
                    ob.data.materials[k] = bpy.data.materials[dst]
                out.setdefault(scope, {}).setdefault(ob.name, []).append(dst)
    log(stage='exportFold', lod=level, folded={s: len(v) for s, v in out.items()})
    return out


def export(path, level=0):
    # `farLampZones` (patrolSedan): the far tier's lamp-zone rules replace `lampZones` at LOD2 only
    far = CFG.get('farLampZones') if level == 2 else None
    lamp_zones(far)
    export_fold(level)   # `exportFold`
    paint_as()
    tier_glass(level)
    for ob in bpy.data.objects:   # `plateCard` tilt: the anchor turns onto its mount only in the written tier
        if '_plateRotX' in ob:
            ob.rotation_euler = (ob['_plateRotX'], 0.0, 0.0)
            del ob['_plateRotX']
    bpy.ops.object.select_all(action='DESELECT')
    bpy.ops.export_scene.gltf(filepath=str(path), export_format='GLB', export_apply=True, export_yup=True,
                              export_animations=False, export_skins=False, export_morph=False,
                              export_extras=True, export_tangents=False)
    paint_rename(path)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--body', required=True, choices=sorted(BODIES))
    parser.add_argument('--blend', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--work', type=Path, default=None, help='where the per-tier .blend snapshots go')
    parser.add_argument('--inner-keep', type=float, default=0.08)
    parser.add_argument('--cage', type=Path, default=None, help="the build's cage.json (cage-built mid/far tiers)")
    parser.add_argument('--parts', type=Path, default=None, help="the build's parts.json (arches, side trims)")
    parser.add_argument('--promote', action='store_true',
                        help='merge the body record into <output>/build-report.json (fleet-v3)')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    global CFG
    CFG = BODIES[args.body]
    ZONE_LENSES.update({n: 'VehicleTaillamp' for n in CFG.get('tailLampNodes', ())})   # `tailLampNodes`
    body = args.body
    args.blend = args.blend.resolve()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    work = (args.work or out).resolve()
    work.mkdir(parents=True, exist_ok=True)

    parts = None
    if CFG.get('cageMid') or CFG.get('cageFar'):
        if not (args.cage and args.parts):
            raise ValueError(f'{body}: cage-built tiers need --cage and --parts')
        args.cage, args.parts = args.cage.resolve(), args.parts.resolve()
        parts = json.loads(args.parts.read_text())
    reduce_source = None
    if CFG.get('planarDeg'):
        # the mid and far reducers start from a shell WITHOUT the near tier's planar merge: a
        # collapse over its long merged triangles smears the cladding zones and folds the panels
        prepare_near(args)
        reduce_source = work / f'{body}-reduce.blend'
        bpy.ops.wm.save_as_mainfile(filepath=str(reduce_source))
    prep = prepare_near(args, planar=bool(CFG.get('planarDeg')))
    near_reductions = budget_near()
    if CFG.get('nearReseatPlates'):
        # `nearReseatPlates` (cityBus): the near budget fit thins the bumper round the plate after the plates were
        # placed (read back 3.17 mm onto Bumper_Front); reseat on the reduced meshes so LOD0 keeps the 3.5 mm stand-off
        prep['platesReseatMm'] = reseat_plates()
    near = work / f'{body}-near.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(near))
    lods, notes = [], {'near': dict(prep, reductions=near_reductions)}
    for level in range(3):
        if level:
            bpy.ops.wm.open_mainfile(filepath=str(reduce_source or near))
        if level == 1 and CFG.get('cageMid'):
            gone = drop(lambda o: mid_engine_bay(o) or o.name.startswith(('Fan_', 'Pedal') + tuple(CFG.get('midDrop', ()))))
            rim = rebuild_rim(*CFG['midRim']) if CFG.get('midRim') else None
            anchors = fix_plate_anchor_names()
            cage = build_cage_tier('cageMid', args, parts)
            cage['plateAnchorNames'] = anchors
            cage['platesMovedMm'] = reseat_plates()
            panels = consolidate_articulated_materials('mid')
            notes['mid'] = dict(dropped=gone, rim=rim, cage=cage, articulated=panels,
                                budget=fit_cage_budget(CAPS[1], cage_mid_table),
                                liningsSeated=seat_cage_linings(CFG['cageMid'].get('lining', 0.02)))
            notes['mid']['seamsFinal'] = finish_seams()
            notes['mid']['linersSeated'] = seat_liners()
            notes['mid']['limitNormals'] = limit_normals(args.cage)
            if CFG.get('reseatDecals'):
                notes['mid']['decalsSeated'] = reseat_decals(CFG['cageMid'].get('decalOffset', 0.004))
        elif level == 2 and CFG.get('cageFar'):
            anchors = fix_plate_anchor_names()
            cage = build_cage_tier('cageFar', args, parts)
            cage['plateAnchorNames'] = anchors
            cage['platesMovedMm'] = reseat_plates()
            gone = drop(lambda o: not cage_far_keep(o))
            for ob in [o for o in bpy.context.scene.objects if o.type == 'EMPTY'
                       and not o.name.startswith(('Wheel_', 'Door_', 'SlideDoor_', 'Hood', 'Trunk'))
                       and not any(c.type == 'MESH' for c in o.children_recursive)]:
                gone.append(ob.name)
                bpy.data.objects.remove(ob, do_unlink=True)
            faces = far_wheel_faces()
            notes['far'] = dict(dropped=len(gone), cage=cage, wheelFaces=faces,
                                budget=fit_cage_budget(CAPS[2], cage_far_table),
                                liningsSeated=seat_cage_linings(CFG['cageFar'].get('glassLining', 0.03)))
            notes['far']['seamsFinal'] = finish_seams()
            notes['far']['linersSeated'] = seat_liners()
            notes['far']['limitNormals'] = limit_normals(args.cage)
            if CFG.get('reseatDecals'):
                notes['far']['decalsSeated'] = reseat_decals(CFG['cageFar'].get('decalOffset', 0.008))
        elif level == 1:
            # the intake meshes are a fine lattice a mid budget can only shred; their dark housings stay
            gone = drop(lambda o: mid_engine_bay(o) or o.name.startswith(('Fan_', 'Pedal', 'Badge') + tuple(CFG.get('midDrop', ())))
                        or (o.name.startswith('Intake_') and o.name.endswith('_Grille')))   # + midDrop
            rim = rebuild_rim(*CFG['midRim']) if CFG.get('midRim') else None
            skins = planar_skins(CFG['midSkinPlanarDeg']) if CFG.get('midSkinPlanarDeg') else None
            if CFG.get('midBodyPlanarDeg'):   # a long flat-sided shell merged by angle, zones kept
                skins = (skins or []) + planar_skins(CFG['midBodyPlanarDeg'], with_body=True)
            if CFG.get('midStripInner'):
                strip_interior(SHELL)   # the bus's 4 mm inner wall made the collapse throw shell vertices 1.7 m up
            if CFG.get('protectZones') and bpy.data.objects.get('Body'):
                bpy.data.objects['Body']['_protectZones'] = True   # vehicle_mid_lod holds the paint-zone outlines
            frit = frit_merge(CFG['tierFritMerge']) if CFG.get('tierFritMerge') else None   # `tierFritMerge`
            notes['mid'] = dict(dropped=gone, rim=rim, skins=skins, reductions=allocate(mid_table, CAPS[1], 4000))
            if frit is not None:
                notes['mid']['frit'] = frit
            for ob in bpy.context.scene.objects:
                if '_planarDone' in ob:
                    del ob['_planarDone']
        elif level == 2:
            gone = drop(lambda o: not far_keep(o))
            # cabin pivots with nothing left under them (the far tier has no cabin; a bare
            # SteeringWheel node would claim a steering rig the tier does not draw)
            for ob in [o for o in bpy.context.scene.objects if o.type == 'EMPTY'
                       and not o.name.startswith(('Wheel_', 'Door_', 'SlideDoor_', 'Hood', 'Trunk'))
                       and not any(c.type == 'MESH' for c in o.children_recursive)]:
                gone.append(ob.name)
                bpy.data.objects.remove(ob, do_unlink=True)
            strip_interior(SHELL)
            if CFG.get('farFlatPrefixes'):
                # `farFlatPrefixes` (cityBus): the flat side panels (skirt flaps, door leaves, access doors) keep a
                # 4 mm inner wall; collapsed to ~40 tris the outer and inner walls merged and the smooth normals
                # averaged across them (up to 170 deg off the face), so the panels shaded light and shiny against the
                # body. Inner wall stripped here, flat shading after the budget fit (far_flat)
                strip_interior([o.name for o in meshes() if o.name.startswith(tuple(CFG['farFlatPrefixes']))])
            faces = far_wheel_faces()
            if CFG.get('farTyreSegments'):
                faces = dict(faces=faces, tyres=far_tyres(CFG['farTyreSegments']))   # `farTyreSegments`
            merged = planar_skins(CFG['farShellPlanarDeg'], with_body=True) if CFG.get('farShellPlanarDeg') else None
            if CFG.get('protectZones') and bpy.data.objects.get('Body'):
                bpy.data.objects['Body']['_protectZones'] = True
            if CFG.get('tierFritMerge'):
                frit_merge(CFG['tierFritMerge'])   # `tierFritMerge`
            for ob in bpy.context.scene.objects:
                if '_planarDone' in ob:
                    del ob['_planarDone']   # the far skins still take their far_table budgets
            notes['far'] = dict(dropped=len(gone), wheelFaces=faces, shellPlanar=merged,
                                reductions=allocate(far_table, CAPS[2], 1800))
            if CFG.get('farFlatPrefixes'):
                notes['far']['flat'] = far_flat(CFG['farFlatPrefixes'])   # `farFlatPrefixes`
        if level and (CFG.get('cageMid') if level == 1 else CFG.get('cageFar')):
            notes['mid' if level == 1 else 'far']['platesFinalMm'] = reseat_plates()   # after the budget merge
        if level and CFG.get('tyreRadiusClamp'):
            notes['mid' if level == 1 else 'far']['tyreRadiusClamp'] = clamp_tyre_radius()
        tris = total_triangles()
        if tris > CAPS[level]:
            raise ValueError(f'LOD{level} over cap {tris} > {CAPS[level]}')
        path = out / f'{body}-lod{level}.glb'
        export(path, level)
        safe = assert_runtime_glb(path)
        lods.append(dict(file=path.name, sha256=digest(path), bytes=path.stat().st_size, triangles=tris))
        notes.setdefault('runtimeSafe', {})[path.name] = safe
        log(stage='export', lod=level, triangles=tris, bytes=path.stat().st_size)
    (work / f'{body}-lods-report.json').write_text(json.dumps(dict(lods=lods, notes=notes), indent=1, default=str) + '\n')
    if args.promote:
        rpath = out / 'build-report.json'
        report = json.loads(rpath.read_text())
        report['bodies'][body] = dict(
            vehicleId=body, body=body, label=CFG['label'], lengthM=CFG['lengthM'],
            wheelRadiusM=CFG['wheelRadiusM'], typeIds=CFG['typeIds'], lods=lods,
            source='scripts/assets/export-skill-vehicle-lods.py',
            sourceModel=dict(
                method='zstack /vehicle skill: hand-shaped mirrored subdivision cage in headless Blender, '
                       'panel cuts, rig and door-swing proof in the skill; no AI 3D generator; '
                       'reference images were look-only and are not part of the asset',
                file=args.blend.name, sha256=digest(args.blend), bytes=args.blend.stat().st_size,
                **({'cage': dict(file=args.cage.name, sha256=digest(args.cage)),
                    'parts': dict(file=args.parts.name, sha256=digest(args.parts))} if parts else {})),
            recipeSha256={name: digest(Path(__file__).with_name(name))
                          for name in (Path(__file__).name, 'vehicle_mid_lod.py')},
            status='candidate; interaction and blind visual review required')
        rpath.write_text(json.dumps(report, indent=2) + '\n')
        log(stage='promote', report=str(rpath))


if __name__ == '__main__':
    main()
