# SF patrol sedan v1

Hand-authored Blender vehicle candidate for EarthOnline, based on the user's
six-view patrol-sedan concept sheet. The vehicle body is derived from the
repository's hand-built `project-sedan-v1` source; no text/image-to-3D geometry
was used.

## Build

Requires Blender 5.1+ and the local `artifacts/vehicles/project-sedan-v1`
source asset from the asset store:

```bash
blender -b artifacts/vehicles/project-sedan-v1/project-sedan-v1.blend \
  --python artifacts/vehicles/sf-patrol-sedan-v1/build_patrol_sedan.py --
```

The default output is `artifacts/vehicles/sf-patrol-sedan-v1/out/`. The script
saves a versioned `.blend`, a GLB candidate, a dimensional/build report, and
front, side, rear, roof, underside, open-door, driver-cabin, and emergency-light review renders.
Set `SF_PATROL_REFERENCE_IMAGE` to the concept image path to pack it into the
Blender file as a non-exported design reference.

The seated driver defaults to the local, currently untracked
`public/assets/human-npc-probe-v3/npc-01.glb` (removed from `main` 2026-11-06; on branch `archive/before-asset-cleanup-2026-11-06`), a CC0 MakeHuman/MPFB development
preview. Set `SF_PATROL_DRIVER_GLB` to another
compatible 56-joint GLB to rebuild with a different licensed source. The script
samples the shared `drive` clip and saves both the source rig and a separate,
removable static seated body. The preview human is not production-authorized.

## Source dimensions

- 1 Blender unit = 1 metre
- Overall length target: 4.95 m
- Blender-source overall width: 2.228 m (including mirrors and spotlight)
- Body width target: 1.90 m (mirrors excluded)
- Overall height target: 1.52 m (lightbar included)
- Wheelbase: 2.90 m
- Wheel radius: 0.35 m
- Source axes: +Z up, -Y forward; GLB export: +Y up, +Z forward

The editable assembly keeps separate wheel, door, window, hood, trunk, light,
and cabin nodes from the source sedan. The patrol finish adds white door skins
and selected front-quarter faces, black stamped-steel wheels with open vents and
small hubcaps, generic six-point star decals, lettering, a low-profile red/blue
lightbar, pushbar, driver spotlight, interior prisoner partition, and mobile-data
terminal. The glazing has a translucent PBR finish and rear defroster lines;
the lightbar has separate colored lenses, reflectors, and LED emitters. The
front seats and floor are fitted to the driver's seated pose.

The GLB round-trip measured 4.950 m long, 2.224 m overall width, and 1.520 m
high. The wheel pivots are 2.900 m apart; each tyre measures 0.701 m in diameter
and rests at the 0.350 m hub height. The 1.900 m body-width contract excludes the
mirrors and external spotlight.

The Blender fit probe measures 0.092 m from the driver's head to the roof
liner, 0.005 m from the seat base to the floor, and 0.053 m from the shoes to
the floor. These clearances do not prove animated entry or collision fit.

Run `node artifacts/vehicles/sf-patrol-sedan-v1/verify_export.mjs` to repeat the
GLB dimension and wheel checks.

## Scope and review status

This is a Blender-source and GLB-candidate stage. The inherited body silhouette,
front-quarter shape, lamps, and generic star are visibly different from the
six-view concept. The close cabin render still shows defects in the preview
officer's hair, hands, and uniform. The project has not connected
these parts to gameplay controls, door/seat occupancy, collision response,
damage, dynamic emergency-light state, or LOD switching. The packaged collision
proxies are authoring references, not runtime physics assets. The repository's
formal AAA visual gate currently records `REJECT`; this asset has not received
the required blind review. Treat the renders as a review candidate, not an
approved AAA claim or an exact replica of a current department fleet vehicle.
The GLB load probe counts 256,050 triangles before LOD work; no game performance
budget has been demonstrated for this candidate.

The GLB articulation probe confirms the door, lid, window, seat, mirror, and
wheel nodes move with the expected travel. Scale-compensator pivots preserve
0.300 m window and 0.150 m seat motion under the vehicle's metric root scale.
