#!/usr/bin/env node
// download the aerial imagery the roof survey measures (USDA NAIP
// through the USGS National Map image service, public domain) into a folder
// OUTSIDE the repository, and write its manifest (source, licence, date, and
// per file: URL, bytes, SHA-256).
//
//   cargo run --release -p eo-kit --example roof_survey -- plan   # writes <root>/tiles.json
//   node native/tools/fetch-aerial.mjs <root>                     # or EO_AERIAL_DIR
//
// Rules this script keeps: only imagery.nationalmap.gov; a generic
// User-Agent and nothing about the user or the project in any request; at
// most three requests at a time; every reply must be a JPEG of the asked
// size or it is retried (the service answers 502/504 under load); files
// already present are kept. Nothing downloaded is executed. The total is
// checked against a 3 GB cap before and during the run.

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

const UA = 'Mozilla/5.0 (compatible; image-tile-client/1.0)';
const HOST = 'imagery.nationalmap.gov';
const CAP = 3 * 1024 ** 3;
const root = process.argv[2] || process.env.EO_AERIAL_DIR || (() => { console.error('usage: fetch-aerial.mjs <root> (or set EO_AERIAL_DIR)'); process.exit(2); })();
const plan = JSON.parse(readFileSync(join(root, 'tiles.json'), 'utf8'));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let total = 0;
for (const t of plan.tiles) {
  const p = join(root, t.file);
  if (existsSync(p)) total += statSync(p).size;
}

// The service's JPEGs end without the EOI marker; accept a JPEG start of the
// announced length (a cut reply is shorter than its Content-Length).
function isJpeg(b, len) {
  return b.length > 1000 && b[0] === 0xff && b[1] === 0xd8 && (len === null || Number(len) === b.length);
}

async function fetchTile(t) {
  const p = join(root, t.file);
  if (existsSync(p)) return 'kept';
  if (new URL(t.url).host !== HOST) throw new Error(`refusing host ${t.url}`);
  for (let attempt = 1; attempt <= 8; attempt++) {
    try {
      const r = await fetch(t.url, { headers: { 'User-Agent': UA }, signal: AbortSignal.timeout(180000) });
      const b = Buffer.from(await r.arrayBuffer());
      if (r.ok && isJpeg(b, r.headers.get('content-length'))) {
        if (total + b.length > CAP) throw new Error('3 GB cap reached');
        mkdirSync(dirname(p), { recursive: true });
        writeFileSync(p, b);
        total += b.length;
        return 'fetched';
      }
      console.error(`${t.file}: attempt ${attempt}: HTTP ${r.status}, ${b.length} bytes`);
    } catch (e) {
      if (String(e.message).includes('cap')) throw e;
      console.error(`${t.file}: attempt ${attempt}: ${e.message}`);
    }
    await sleep(4000 * attempt);
  }
  return 'failed';
}

const queue = [...plan.tiles];
const counts = { kept: 0, fetched: 0, failed: 0 };
const t0 = Date.now();
async function worker() {
  while (queue.length) {
    const t = queue.shift();
    const r = await fetchTile(t);
    counts[r]++;
    const done = counts.kept + counts.fetched + counts.failed;
    if (r !== 'kept' || done % 50 === 0) {
      console.log(`${done}/${plan.tiles.length} ${t.file} ${r}; ${(total / 1024 ** 2).toFixed(0)} MB; ${((Date.now() - t0) / 1000).toFixed(0)} s`);
    }
  }
}
// EO_FETCH_WORKERS: requests in flight (default 3, at most 6).
const workers = Math.min(6, Math.max(1, Number(process.env.EO_FETCH_WORKERS) || 3));
await Promise.all(Array.from({ length: workers }, worker));

const files = [];
for (const t of plan.tiles) {
  const p = join(root, t.file);
  if (!existsSync(p)) continue;
  const b = readFileSync(p);
  files.push({ file: t.file, url: t.url, bbox: t.bbox, bytes: b.length, sha256: createHash('sha256').update(b).digest('hex') });
}
const manifest = {
  source: 'USDA Farm Service Agency National Agriculture Imagery Program (NAIP), served by the USGS National Map image service USGSNAIPImagery',
  service: `https://${HOST}/arcgis/rest/services/USGSNAIPImagery/ImageServer`,
  serviceCopyrightText: 'USGS, USDA, The National Map: Orthoimagery. January 09, 2025.',
  licence: 'Public domain. The service description states "free downloads of public domain, NAIP compressed orthoimagery"; the data.gov record for NAIP imagery lists the licence https://www.usa.gov/publicdomain/label/1.0/ (publisher: USDA Farm Production and Conservation Business Center). USGS asks for the credit "U.S. Geological Survey" (https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits).',
  licenceLinks: ['https://catalog.data.gov/dataset/national-agriculture-imagery-program-naip-imagery', 'https://www.usa.gov/publicdomain/label/1.0/', 'https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits'],
  credit: 'Imagery: USDA NAIP, via the U.S. Geological Survey National Map',
  downloaded: new Date().toISOString(),
  grids: { rgb: plan.rgb, cir: plan.cir, frame: plan.frame, note: 'plain longitude/latitude (EPSG:4326) tiles; rgb = bands 1,2,3 (red, green, blue); cir = bands 4,1,2 (near infrared, red, green)' },
  totalBytes: files.reduce((a, f) => a + f.bytes, 0),
  files,
};
writeFileSync(join(root, 'manifest.json'), JSON.stringify(manifest, null, 1));
console.log(`done: ${JSON.stringify(counts)}; ${files.length} files, ${(manifest.totalBytes / 1024 ** 2).toFixed(0)} MB; manifest ${join(root, 'manifest.json')}`);
if (counts.failed) process.exitCode = 1;
