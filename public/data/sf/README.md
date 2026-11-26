# San Francisco data

The files in this directory are tracked in Git and read by the game at runtime.
They were produced by the project's earlier data pipeline; those generator
scripts are not part of this repository, but each file records its generator,
inputs and digests in its own metadata or in the NOTICE file beside it. Keep the
NOTICE files with the data when you redistribute it.

| File | What it is | Licence | Notice |
| --- | --- | --- | --- |
| `sf-city.json.gz` | Roads, OSM building footprints, signals, sidewalks, crossings, street points, rail ways and the land boundary, in world metres | ODbL 1.0 (© OpenStreetMap contributors); shoreline and civic street attributes from DataSF under PDDL 1.0; USGS 3DEP (public domain) for bridge ends | `sf-city.NOTICE.txt`; sources in `meta.sources` |
| `sf-footprint-supplement.json` | 276 extra building footprints with address, height and assessor attributes | PDDL 1.0 (DataSF Building Footprints and Assessor roll) | `sf-footprint-supplement.NOTICE.txt` |
| `sf-footprints.json.gz` | Citywide building footprints and heights the OSM extract leaves out | PDDL 1.0 (DataSF Building Footprints) | `sf-footprints.NOTICE.txt` |
| `sf-elevation.bin.gz` | Ground and seabed elevation, 8 m grid, NAVD88 | Public domain (USGS 3DEP, NOAA NCEI CUDEM) | `sf-elevation.NOTICE.txt` |
| `sf-land-cover.bin.gz` | Land-cover classes for unmapped open ground | CC BY 4.0 (ESA WorldCover), attribution required | `sf-land-cover.NOTICE.txt` |
| `sf-lots-parks.json.gz` | Surface car parks, green areas and inland water | ODbL 1.0 (© OpenStreetMap contributors) | `sf-lots-parks.NOTICE.txt` |
| `sf-map-citywide.json.gz` | Citywide map cartography (pause map, corner map, GPS) | ODbL 1.0, with PDDL 1.0 and public-domain inputs | `sf-map-citywide.NOTICE.txt` |
| `sf-storm-drains.json.gz` | Street storm drains | PDDL 1.0 (DataSF / SFPUC) | `sf-storm-drains.NOTICE.txt` |
| `sf-tree-sizes.json.gz` | Street trees: position, species, trunk diameter (DBH), plot, site, planting date; built by `scripts/world-data/build-sf-tree-sizes.py` | PDDL 1.0 (DataSF / SF Public Works) | `sf-tree-sizes.NOTICE.txt` |
| `sf-tree-allometry.json` | Urban Tree Database equations (crown diameter and height from DBH) for the list's species, and each species' match; built by the same script | U.S. Government work (USDA Forest Service RDS-2016-0005), citation requested | `sf-tree-allometry.NOTICE.txt` |
| `sf-street-survey.json.gz` | Kerbside parking regulation stretches (allowed or banned) and on-street meters, and the mid-block signals, beacons and flashers, in world metres (built by `scripts/world-data/build-sf-street-survey.py`) | PDDL 1.0 (DataSF / SFMTA) | `sf-street-survey.NOTICE.txt` |
| `sf-bikeways.json.gz` | Each street segment's bike facilities (class, direction, contraflow, buffer, raised, barrier, green paint) with their lines, in world metres (built by `scripts/world-data/build-sf-bikeways.py`) | PDDL 1.0 (DataSF / SFMTA) | `sf-bikeways.NOTICE.txt` |
| `sf-pavement.json.gz` | Each street segment's Pavement Condition Index and latest resurfacing date, with its line, in world metres (built by `scripts/world-data/build-sf-pavement.py`) | PDDL 1.0 (DataSF / SF Public Works) | `sf-pavement.NOTICE.txt` |
| `building-metadata/v1/` | Per-building use and ground-floor unit categories for the extract's buildings (SoMa pilot areas and citywide) | DataSF PDDL 1.0, OSM ODbL 1.0, SF Planning CC0 1.0 | sources in its `index.json` |
| `building-metadata/v1-pack/` | The same categories citywide, including the DataSF footprint-pack lots | DataSF PDDL 1.0, OSM ODbL 1.0, SF Planning CC0 1.0 | sources in its `index.json` |

The building-metadata packs store categories only: no business names, brands
or operators. Inputs, their sha256 digests and licences are listed in each
pack's `index.json`.

Files derived from OpenStreetMap are Derivative Databases under the Open
Database License 1.0 (https://opendatacommons.org/licenses/odbl/1-0/). Any
public use must keep the "© OpenStreetMap contributors" attribution and share
derivative databases under the ODbL.
