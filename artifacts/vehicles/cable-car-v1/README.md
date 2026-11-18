# cable-car-v1

Reference set for an original open-sided cable tram streetcar, generated with the
built-in ImageGen tool. The vehicle catalogue in `src/vehicles/vehicle-catalogue.js`
has room for this body and currently has nothing in it: 20 bodies are registered
(compact hatch, sedan, coupe, wagon, crossover, pickup, panel van, box truck,
transit bus, taxi, patrol sedan, compact minivan, full-size SUV, four trims and the
autonomous two-door cab), and no rail-guided or cable-hauled vehicle is among them.
`src/world.js` already draws a static cable car on California Street out of
`RoundedBoxGeometry` primitives; this is the authored body that replaces it.

## Files

`refs/` - seven views of one consistent subject, all 1536x1024 landscape except
where noted:

| file | view | use |
| --- | --- | --- |
| `cablecar_front34.png` | front three-quarter, camera above eye level | hero/reference view |
| `cablecar_side.png` | exact left side elevation, profile-parallel (1760x894) | the dimension master |
| `cablecar_rear.png` | exact rear elevation (1374x1145) | tail and rear platform |
| `cablecar_front.png` | exact front elevation | both running boards in one frame |
| `cablecar_over34.png` | high three-quarter from above-left | underframe and grip mechanism |
| `cablecar_interior.png` | along the open deck toward the front platform | benches, ceiling, crew lever |

## Design intent

A two-axle grip car for a cable-hauled line. Local frame is the runtime convention:
+Y up, +Z forward, one unit per metre, one metre equals one Blender unit.

- Overall: 7.7 m long, 1.9 m wide body, 3.2 m tall to the roof crown.
- Wheels: 0.45 m radius, steel spokes, two axles.
- Body: deep carmine red riveted panels, cream ivory window band and roof trim.
- Interior: varnished oak benches, deck, ceiling and uprights; brass handrails,
  stanchion caps, pendant lamps and trim; a crew lever at the front platform.
- The flanks are open, not glazed: oak uprights roughly every 0.8 m with a
  continuous brass handrail at waist height, plus a running board at sill height
  on each side with a folded step.
- Roof: full length with a slight crown and a clerestory glazing strip each side.
- Underframe: chassis rails, leaf springs, drive chains, brake linkages and a
  grip assembly on a transverse shaft near mid-wheelbase, gripping the cable in
  the slot between the running rails.

No marque, model, operator name, route number, destination sign, livery text,
number plate or badge appears in any view, per the naming policy in CLAUDE.md.

## Provenance

Synthesised from text prompts only. No source pixels, no third-party mesh, no
trademarked or photographed reference. Paints are plain hex values, not a
measured colour match. Every figure above is a plausible class dimension, not a
certified spec, and vertical status is unverified until the model exists.

## Next step

The workflow this feeds is the existing authored route, not a new pipeline:

1. Derive a station table in the shape of the catalogue's `profileOverride` or
   `scripts/assets/vehicle-v3` body recipe, driven by the side elevation.
2. Build with `scripts/assets/build-vehicle-fleet-v2.py`-style headless Blender so
   the wheel pivots, door keys and bake transforms follow the fleet contract.
3. Register the body in `VEHICLE_SPECS` and `TRAFFIC_KIND_MAP` (or leave the spawn
   weight at zero, as the pickup, panel van and box truck currently do, until the
   loft survives close-up review).

`refs/` is local working material and is never imported by the runtime.
