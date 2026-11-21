#!/usr/bin/env node
// The survey set's provenance manifest, native/assets/survey-manifest.json:
// every file the owner-approved survey folder holds, re-hashed from disk
// (path relative to the root, bytes, SHA-256), grouped as the download
// manifest groups them, with each group's publisher, licence and attribution.
// The game reads the folder from EO_SURVEY_ROOT (no default) through
// eo-farshore's survey readers, which refuse a file whose size or SHA-256
// differs from this manifest.
//
//   node native/tools/survey-manifest.mjs [root]      # or EO_SURVEY_ROOT
//
// Inputs under the root: MANIFEST.json (groups E1, B1, S1, S2 and the lidar
// groups L*, whose files were not fetched) and A1/manifest.json (the NAIP
// tiles and their grid). Nothing is downloaded; a file whose hash differs from
// the download manifest's stops the script.

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, statSync, writeFileSync, openSync, readSync, closeSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(here, '..', 'assets', 'survey-manifest.json');
const root = resolve(process.argv[2] || process.env.EO_SURVEY_ROOT || (() => { console.error('usage: survey-manifest.mjs <root> (or set EO_SURVEY_ROOT)'); process.exit(2); })());
const read = (p) => JSON.parse(readFileSync(join(root, p), 'utf8'));

function sha256(path) {
  const h = createHash('sha256');
  const fd = openSync(path, 'r');
  const buf = Buffer.allocUnsafe(8 << 20);
  try {
    for (;;) {
      const n = readSync(fd, buf, 0, buf.length, null);
      if (n === 0) break;
      h.update(buf.subarray(0, n));
    }
  } finally {
    closeSync(fd);
  }
  return h.digest('hex');
}

// What each group is and how it is credited (the publishers' own wording where they give one).
const GROUPS = {
  A1: {
    what: 'USDA NAIP 0.6 m RGB aerial imagery over the Golden Gate photo spots, plain longitude/latitude (EPSG:4326) JPEG tiles',
    publisher: 'USDA Farm Service Agency, National Agriculture Imagery Program (NAIP), served by the USGS National Map image service USGSNAIPImagery',
    licence: 'Public domain (U.S. Government work; https://www.usa.gov/publicdomain/label/1.0/)',
    attribution: 'Imagery: USDA NAIP, via the U.S. Geological Survey National Map',
  },
  E1: {
    what: 'USGS 3D Elevation Program 1 m DEM GeoTIFFs (UTM zone 10N NAD83, NAVD88 metres)',
    publisher: 'U.S. Geological Survey, The National Map',
    licence: 'Public domain (U.S. Government work)',
    attribution: 'U.S. Geological Survey, 3D Elevation Program',
  },
  B1: {
    what: 'NOAA NCEI Continuously Updated Digital Elevation Model (CUDEM) 1/9 arc-second topobathy GeoTIFFs (NAD83 geographic, NAVD88 metres)',
    publisher: 'NOAA National Centers for Environmental Information',
    licence: 'Public domain (U.S. Government work)',
    attribution: 'NOAA National Centers for Environmental Information, CUDEM',
  },
  S1: {
    what: 'Poly Haven scanned rock models (glTF, 2k diffuse, normal and ARM maps)',
    publisher: 'Poly Haven (polyhaven.com)',
    licence: 'CC0 1.0 (https://creativecommons.org/publicdomain/zero/1.0/)',
    attribution: 'Poly Haven (polyhaven.com), CC0; no attribution required',
  },
  S2: {
    what: 'Poly Haven scanned grass and shrub models (glTF, 2k diffuse, normal and ARM maps)',
    publisher: 'Poly Haven (polyhaven.com)',
    licence: 'CC0 1.0 (https://creativecommons.org/publicdomain/zero/1.0/)',
    attribution: 'Poly Haven (polyhaven.com), CC0; no attribution required',
  },
};

function hashed(rel, expect) {
  const abs = join(root, rel);
  if (!existsSync(abs)) throw new Error(`${rel}: listed but not on disk`);
  const bytes = statSync(abs).size;
  const digest = sha256(abs);
  if (expect.bytes !== undefined && expect.bytes !== bytes) throw new Error(`${rel}: ${bytes} bytes, the download manifest says ${expect.bytes}`);
  if (expect.sha256 && expect.sha256 !== digest) throw new Error(`${rel}: sha256 ${digest}, the download manifest says ${expect.sha256}`);
  return { path: rel, bytes, sha256: digest, ...(expect.url ? { url: expect.url } : {}) };
}

const top = read('MANIFEST.json');
const naip = read('A1/manifest.json');
const sets = [];
sets.push({
  group: 'A1',
  id: 'naip-rgb',
  ...GROUPS.A1,
  complete: true,
  grid: naip.grids.rgb,
  files: naip.files.map((f) => hashed(`A1/${f.file}`, f)),
});
const lidar = [];
for (const it of top.items) {
  if (it.group.startsWith('L')) {
    lidar.push({ group: it.group, file: it.file, publisher: it.publisher, licence: it.license, complete: false });
    continue;
  }
  const g = GROUPS[it.group];
  if (!g) throw new Error(`unknown group ${it.group}`);
  if (!it.complete) throw new Error(`${it.group} ${it.file}: incomplete in the download manifest`);
  sets.push({ group: it.group, id: it.file, ...g, complete: true, files: it.files.map((f) => hashed(f.path, f)) });
}
const total = sets.reduce((s, x) => s + x.files.reduce((t, f) => t + f.bytes, 0), 0);
const manifest = {
  schema: 'eo-survey-manifest-v1',
  generatedBy: 'native/tools/survey-manifest.mjs',
  rootEnv: 'EO_SURVEY_ROOT',
  note: 'Owner-approved survey and scan files for the Golden Gate area, kept outside the repository; paths are relative to the root. eo-farshore survey readers check every file they open against bytes and sha256 here.',
  downloadNote: top.note,
  totalBytes: total,
  sets,
  notFetched: {
    reason: 'The USGS lidar point clouds (groups L*) were not fetched: the server answered HTTP 429 to every request (download manifest note).',
    files: lidar,
  },
};
writeFileSync(out, JSON.stringify(manifest, null, 1) + '\n');
console.log(`wrote ${out}: ${sets.length} sets, ${sets.reduce((s, x) => s + x.files.length, 0)} files, ${(total / 1e6).toFixed(1)} MB; ${lidar.length} lidar files recorded as not fetched`);
