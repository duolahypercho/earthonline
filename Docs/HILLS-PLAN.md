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
- Rock: procedural blocks, "sandstone" outside two boxes.
- Structures in the survey: the bare-earth DEM keeps some roof decks as
  ground. Moscone South (4th St, Howard-Folsom) becomes a 6-11 m earth mound.

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
| H3 | 1 m relief on open ground citywide (generalise `terrain_detail` regions) | planned |
| H4 | Structure cleanup in the ground (roof decks, Moscone South) | planned |
| H5 | Surface map as material IDs in the terrain shader | planned |
| H6 | Vegetation from the layers (grass by cure, coyote brush, lidar trees) | planned |
| H7 | Rock by lithology (chert on Twin Peaks, Bernal, Mount Davidson) | planned |

Each stage ships with two matched capture sets (before, after), the bench
cost per path, and a kill switch.
