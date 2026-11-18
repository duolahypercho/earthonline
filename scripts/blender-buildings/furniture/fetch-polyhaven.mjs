// Download the curated CC0 furniture set from Poly Haven into the off-repo
// data store (never into the working tree), with author/licence records.
//
//   node scripts/blender-buildings/furniture/fetch-polyhaven.mjs --root /path/to/polyhaven-store
//   (or set EO_POLYHAVEN_STORE; the store must live outside the working tree)
//
// The curated list lives in ./catalog.json; prepare_library.py turns the
// downloads into small, game-ready pieces for build_building.py.
import { createWriteStream, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { pipeline } from 'node:stream/promises';
import { Readable } from 'node:stream';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '..', '..', '..');
const args = process.argv.slice(2);
const rootArg = args.includes('--root') ? args[args.indexOf('--root') + 1] : process.env.EO_POLYHAVEN_STORE;
if (!rootArg) throw new Error('pass --root <dir> or set EO_POLYHAVEN_STORE to the off-repo Poly Haven store');
const root = path.resolve(rootArg);
if (!path.relative(REPO, root).startsWith('..')) throw new Error('the Poly Haven store must live outside the working tree');
const catalog = JSON.parse(readFileSync(path.join(HERE, 'catalog.json'), 'utf8'));
const UA = { 'User-Agent': 'earth-online-asset-fetch/1.0' };

async function retry(run, label) {
  for (let attempt = 1; ; attempt += 1) {
    try { return await run(); } catch (error) {
      if (attempt >= 5) throw error;
      console.warn(`${label}: ${error.cause?.code || error.message}, retry ${attempt}`);
      await new Promise((resolve) => setTimeout(resolve, 1500 * attempt));
    }
  }
}

async function json(url) { return retry(() => jsonOnce(url), url); }

async function jsonOnce(url) {
  const response = await fetch(url, { headers: UA });
  if (!response.ok) throw new Error(`${url} -> HTTP ${response.status}`);
  return response.json();
}

async function download(url, target) {
  if (existsSync(target)) return;
  await retry(() => downloadOnce(url, target), url);
}

async function downloadOnce(url, target) {
  mkdirSync(path.dirname(target), { recursive: true });
  const response = await fetch(url, { headers: UA });
  if (!response.ok) throw new Error(`${url} -> HTTP ${response.status}`);
  const partial = `${target}.part`;
  await pipeline(Readable.fromWeb(response.body), createWriteStream(partial));
  renameSync(partial, target);
}

for (const entry of catalog.assets) {
  const id = entry.polyhaven;
  const dir = path.join(root, id);
  const files = await json(`https://api.polyhaven.com/files/${id}`);
  const info = await json(`https://api.polyhaven.com/info/${id}`);
  const gltf = files.gltf?.['1k']?.gltf;
  if (!gltf) { console.warn(`${id}: no 1k glTF, skipped`); continue; }
  await download(gltf.url, path.join(dir, path.basename(gltf.url)));
  for (const [rel, file] of Object.entries(gltf.include || {})) await download(file.url, path.join(dir, rel));
  writeFileSync(path.join(dir, 'source.json'), `${JSON.stringify({
    source: `https://polyhaven.com/a/${id}`,
    license: 'CC0-1.0',
    authors: Object.keys(info.authors || {}),
    name: info.name,
    polycount: info.polycount ?? null,
    dimensionsMm: info.dimensions ?? null,
    gltf: path.basename(gltf.url),
  }, null, 2)}\n`);
  console.log(`${id}: ${info.name} by ${Object.keys(info.authors || {}).join(', ')} (${info.polycount} tris)`);
}
// surface textures: 1k diffuse / ARM / GL normal, into <root>/../polyhaven-textures/<id>/
const textureRoot = path.join(path.dirname(root), 'polyhaven-textures');
for (const id of catalog.textures || []) {
  const files = await json(`https://api.polyhaven.com/files/${id}`);
  const info = await json(`https://api.polyhaven.com/info/${id}`);
  for (const key of ['Diffuse', 'arm', 'nor_gl']) {
    const file = files[key]?.['1k']?.jpg;
    if (file) await download(file.url, path.join(textureRoot, id, path.basename(file.url)));
  }
  writeFileSync(path.join(textureRoot, id, 'source.json'), `${JSON.stringify({
    source: `https://polyhaven.com/a/${id}`, license: 'CC0-1.0', authors: Object.keys(info.authors || {}), name: info.name,
  }, null, 2)}\n`);
  console.log(`texture ${id}: ${info.name}`);
}
console.log(`store: ${root}`);
