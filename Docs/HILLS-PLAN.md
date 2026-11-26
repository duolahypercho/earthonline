# Hills: photoreal open ground across the city

Status: in progress on branch `hills/r1`. Nothing here is merged; every
feature sits behind its own `--no-<feature>` switch.

## Goal

Every hill and open slope in the playable city reads as the real place, on
foot and from the air: Twin Peaks, Bernal Heights, Mount Davidson, Corona
Heights, Telegraph Hill, Glen Canyon, McLaren Park, Bayview Hill, Lands End,
the Presidio and the city's sloped parks. The bar is a photograph of the
same place, judged under [VISUAL_QUALITY_GATE.md](VISUAL_QUALITY_GATE.md).

## What draws a hill today (2026-11-25)

- Shape: the 8 m grid of `public/data/sf/sf-elevation.bin.gz`, scaled 1.25.
  The 2 m relief of `eo-render/src/terrain_detail.rs` covers only 400 m
  round five Golden Gate photo eyes.
- Ground colour: per 8 m vertex, guessed from the 16 m NAIP green share and
  WorldCover (`eo-render/src/ground2.rs`). The shaders then guess the surface
  type back from that colour (`cc0_ground_sets`, `nt_cover_weight`,
  `nt_turf_weight`).
- Grass: the gg2 vegetation's meadow patches (`eo-landmarks/src/vegetation.rs`
  `plant_meadow`). Each patch lays an opaque 1.7 m disc ("mat") under its
  blades, shaded apart from the ground and darkened to 0.6 within 5-26 m.
  On a slope these read as dark plates with hard edges.
- Trees: generic models on a 14 m lattice inside OSM greens.
- Rock: procedural blocks, "sandstone" outside two boxes (H7: by the city's
  bedrock map where it maps bedrock).
- Structures in the survey: the bare-earth DEM keeps some roof decks as
  ground. Moscone South (4th St, Howard-Folsom) becomes a 6-11 m earth mound
  (H4: brought down to the streets' grade).

## The hill builder

An offline cook that bakes, per terrain tile, from open data only:

1. Clean ground at 1 m: the USGS 3DEP 1 m DEM (all six tiles are local in
   `external/dem/1m`, digests equal to the shipped grid's header), with roof
   decks and structures cut back to street grade and their edges kept as
   walls.
2. A surface map: lawn, dry grassland, coastal scrub, forest floor,
   trail or bare soil, rock outcrop, paving. Inputs: NAIP 0.3 m RGB and
   0.6 m CIR (`../naip-aerial-sq`, registered to about 1 m), OSM paths and
   greens, WorldCover, slope and aspect, USGS geologic maps for rock type,
   lidar where fetched.
3. Placement lists: trees from lidar (height, crown), scrub clumps, grass
   density, height and cure, rock outcrops by lithology.

The runtime reads these instead of guessing: the terrain takes its material
from the surface map and its colour from the de-lit aerial macro map; the
grass, scrub, tree and rock systems take their placements from the lists.

## Stages

| Stage | What | Status |
| --- | --- | --- |
| H0 | Matched capture set (nine hills on foot and from the air, the Moscone mound), A/B tooling | done |
| H1 | Meadow mats: no plates (the cover blends into the ground); `--no-mat-free-meadow` | done |
| H2 | Aerial macro colour: NAIP classified into lawn, cured grass, soil, paving, scrub, canopy, each in the renderer's own tone with the photo's pattern; `--no-aerial-ground` | done (first pass) |
| H3 | 1 m relief on open ground citywide: the hill builder's cooked high-pass drawn as 2 m patches over the hills and wild parks; `--no-city-relief` | done (first pass) |
| H4 | Roof decks out of the ground: the decks the bare-earth survey keeps as ground, listed by hand from the hill builder's `cook decks` (Moscone Center South's roof: 5-9 m over 4th and Folsom Streets, drawn as an earth mound with 25 m banks), their 8 m cells brought down to the streets' grade by a harmonic fill (`native/assets/sf-decks-v1.json`, eo-data `decks`, applied as the grid loads, so the terrain, collision, streets, buildings and planting agree); the deck's garden is not yet drawn raised on a podium; `--no-deck-ground` | first pass |
| H5 | Surface map as material IDs in the terrain shader | planned |
| H6a | The planting's NAIP rules citywide from the hill builder's class-count map; parks with cured ground are grassland, not lawn; `--no-city-planting` | done (first pass) |
| H6b | Grass that reads as a canopy on foot: the city's open grassland as a standing annual canopy (wild oats and the other annual grasses 0.4-0.9 m with seed heads over their litter, darker down in the canopy; a floor darkened near the eye was tried and rejected: a camera-centred halo); `--no-city-grass` | first pass |
| H6c | Trees where the photograph has them: the greens' 14 m park-tree lattice, near and far, only under the NAIP image's canopy (the hill builder's planting map, the coastal trees' rule), so the open grassland of McLaren and Bernal carries no tree the image does not show (`--no-city-canopy-trees`); next, their height and crown from lidar | first pass (canopy gate) |
| H7 | Rock by lithology: the hill rock and the planting read the city's bedrock map (`native/assets/sf-lithology-v1.json.gz`, the USGS geologic map units of MF-2337, OF 98-354 and MF-2342 as rock classes): chert on Twin Peaks, Mount Davidson, Bernal Heights, Corona Heights and Tank Hill, serpentinite on Potrero Hill, greenstone where mapped; Quaternary deposits and unmapped ground keep the boxes; `--no-city-lithology` | first pass |

Each stage ships with two matched capture sets (before, after), the bench
cost per path, and a kill switch.
