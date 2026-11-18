# SF police helicopter v1

A generic light single-engine utility helicopter in a police air-support
livery, hand-authored as a headless Blender script. No text/image-to-3D
generator touched it: the fuselage is lofted through superellipse
cross-sections and smoothed by subdivision; the tail, skids, rotors,
searchlight and lamps are modelled parts placed from numbers.

## Build

```bash
blender -b --factory-startup \
  --python artifacts/vehicles/sf-police-helicopter-v1/build_police_helicopter.py -- \
  [out_dir] [--no-renders]
node artifacts/vehicles/sf-police-helicopter-v1/install_runtime.mjs
```

The Blender step writes `out/sf-police-helicopter-v1.{blend,glb}`, five review
renders in `out/renders/` and `out/build-report.json` (dimensions, triangle and
object counts, named nodes). The installer copies the GLB to
`public/assets/vehicles/police-helicopter-v1/`, writes `provenance.json` beside
it and regenerates `src/vehicles/police-helicopter-manifest.js` with the
SHA-256 the runtime verifies before it parses the file. Never hand-edit that
digest; rebuild and rerun the installer.

`out/*.glb`, `out/*.blend` and the renders are pipeline binaries (gitignored);
only this README, the scripts and `out/build-report.json` are tracked.

## Frame and named nodes

1 Blender unit = 1 m, +Z up, nose toward -Y; the glTF export turns that into
+Y up, nose toward +Z, skids at y = 0. Overall 10.36 m rotor span x 12.8 m
long x 3.37 m high.

| node | runtime use (src/app/police-helicopter-runtime.js) |
| --- | --- |
| `MainRotor` | spins about its local up axis; a faint blur disc is added |
| `TailRotor` | spins about its local X axis |
| `Searchlight` | gimbal; aimed at the beam point every step (its +Z is the beam) |
| `Light_Beacon_Top`, `Light_Beacon_Belly` | red rotating beacons |
| `Light_Nav_L`, `Light_Nav_R` | red / green position lights |
| `Light_Strobe_Tail` | white anti-collision strobe |

## License

Original work authored for EarthOnline.
