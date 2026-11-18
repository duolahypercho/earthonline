#!/usr/bin/env node
// download the CC0 texture sets the native renderer reads (Poly Haven and
// ambientCG only, CC0 1.0) into a folder OUTSIDE the repository and write the
// provenance manifest `native/assets/cc0-manifest.json` (per file: source,
// asset id, page, authors as the site lists them, licence, resolution,
// download URL, bytes, SHA-256).
//
//   node native/tools/fetch-cc0.mjs --root <folder> [--dry-run] [--manifest-only]   # or EO_CC0_ROOT
//
// Rules this script keeps: only api.polyhaven.com / dl.polyhaven.org and
// ambientcg.com; a generic User-Agent and nothing about the user or the
// project in any request; one request at a time with a pause between assets;
// every archive extracted into its own new empty folder with the system's
// archive tool; nothing downloaded is ever executed. Re-running skips files
// already present with the size the API reports.
//
// The renderer reads the root from EO_CC0_ROOT (no default)
// and refuses any file whose SHA-256 differs from the manifest.

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const UA = 'cc0-asset-fetch/1.0';
const here = dirname(fileURLToPath(import.meta.url));
const manifestPath = resolve(here, '..', 'assets', 'cc0-manifest.json');
const argv = process.argv.slice(2);
const flag = (k) => argv.includes(k);
const opt = (k, d) => {
  const i = argv.indexOf(k);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : d;
};
const root = resolve(opt('--root', process.env.EO_CC0_ROOT) || (() => { console.error('usage: fetch-cc0.mjs --root <folder> (or set EO_CC0_ROOT)'); process.exit(2); })());
const BUDGET_BYTES = 3.0e9;

// What each set is used for (the renderer's own table in eo-assets/src/cc0.rs
// maps sets to material classes; this is the human-readable reason).
const POLYHAVEN = [
  ['white_stucco', 'stucco walls (painted-stucco, float-finish-stucco), fresh paint'],
  ['worn_plaster_wall', 'stucco walls, worn (fetched; not drawn: its peeled patches read as camouflage under paint)'],
  ['red_bricks_04', 'red brick, running bond'],
  ['brick_wall_006', 'red-brown brick, common bond'],
  ['yellow_bricks', 'buff brick'],
  ['white_sandstone_blocks_02', 'cast stone and limestone ashlar'],
  ['concrete_wall_008', 'smooth concrete and precast panels'],
  ['wood_textured_concrete', 'board-formed concrete (landmark piers, seawall faces)'],
  ['corrugated_iron_02', 'corrugated metal cladding'],
  ['box_profile_metal_sheet', 'standing-seam and box-profile metal panels'],
  ['rectangular_facade_tiles', 'shopfront glazed tile'],
  ['concrete_block_wall', 'painted concrete block'],
  ['painted_metal_shutter', 'sectional and roll-up garage doors'],
  ['tarred_gravel', 'flat tar-and-gravel roofs'],
  ['clay_roof_tiles_02', 'clay barrel-tile roofs'],
  ['worn_asphalt', 'carriageway wear in the wheel paths'],
  ['road_damaged', 'cracked carriageway'],
  ['clean_asphalt', 'fresh utility-cut patches'],
  ['dirty_concrete', 'sidewalk concrete'],
  ['concrete_floor_worn_001', 'sidewalk concrete, worn'],
  ['granite_wall', 'granite kerbs (fetched; not drawn: its block joints read as a dashed band on a 15 cm kerb top)'],
  ['herringbone_pavement', 'plaza brick pavers'],
  ['patterned_concrete_pavers', 'plaza concrete pavers'],
  ['cobblestone_floor_08', 'plaza granite setts'],
  ['leafy_grass', 'grass with leaf litter: under trees, gardens and yards'],
  ['withered_grass', 'dry grass on open hills'],
  ['park_dirt', 'park soil and paths'],
  ['gravel_floor_03', 'gravel paths and yards'],
  ['forest_floor', 'ground under trees'],
  ['coast_sand_04', 'dry beach sand'],
  ['damp_beach_sand', 'wet beach sand'],
  ['seaside_rock', 'riprap and shore rock (fetched; not drawn: no surface of the port is riprap)'],
];
// [id, resolution, use]
const AMBIENTCG = [
  ['WoodSiding009', '2K-JPG', 'painted clapboard, fresh paint'],
  ['Planks010', '2K-JPG', 'painted clapboard, weathered paint'],
  ['WoodSiding011', '2K-JPG', 'shingle siding (cedar and painted shingle)'],
  ['Grass004', '2K-JPG', 'irrigated park lawn'],
  ['AsphaltDamage001', '2K-JPG', 'decal: carriageway cracks'],
  ['ManholeCover004', '1K-JPG', 'decal: manhole cover (fetched; not drawn: its cobbled surround is not a San Francisco street)'],
  ['ManholeCover003', '1K-JPG', 'decal: utility vault cover (concrete-filled) beside the kerb'],
  ['ChewingGum001', '1K-JPG', 'decal: gum spots on the sidewalk'],
  ['Leaking001', '1K-JPG', 'wall stain below sills'],
  ['Leaking003', '1K-JPG', 'wall stain below sills'],
  // cc0 v2: street and wall wear decals.
  ['ManholeCover001', '1K-JPG', 'decal: cast-iron sewer manhole cover on a lane centre'],
  ['ManholeCover006', '1K-JPG', 'decal: cast-iron manhole cover (second pattern)'],
  ['ChewingGum002', '1K-JPG', 'decal: gum spots on busy sidewalks (second scatter)'],
  ['TireTracks001', '2K-JPG', 'decal: tyre tread imprint, as a mask for rubber tracks in wheel paths and parking lanes'],
  ['SurfaceImperfections002', '1K-JPG', 'decal: stain blotches, as the mask of oil drips and leaks on the carriageway'],
  ['SurfaceImperfections008', '1K-JPG', 'overlay: mottled dirt inside each sidewalk slab and over the carriageway'],
  ['Leaking008', '1K-JPG', 'decal: grime rising from the foot of walls and kerb faces, and along the gutter'],
  ['Grate002', '1K-JPG', 'decal: storm-drain inlet grate in the gutter'],
  ['RoadLines004', '1K-JPG', 'mask: worn-paint pattern inside the geometric lane markings (never marking geometry)'],
  ['RoadLines001', '1K-JPG', 'worn-paint candidate (fetched; not drawn: RoadLines004 laid along each marking serves every width)'],
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function getJson(url) {
  const r = await fetch(url, { headers: { 'User-Agent': UA, Accept: 'application/json' } });
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return r.json();
}

async function download(url, path, expectBytes) {
  if (existsSync(path) && (!expectBytes || statSync(path).size === expectBytes)) return false;
  if (flag('--dry-run')) return false;
  mkdirSync(dirname(path), { recursive: true });
  const r = await fetch(url, { headers: { 'User-Agent': UA } });
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  const buf = Buffer.from(await r.arrayBuffer());
  if (expectBytes && buf.length !== expectBytes) throw new Error(`${url}: ${buf.length} B, the API said ${expectBytes}`);
  writeFileSync(path, buf);
  return true;
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function rel(p) {
  return relative(root, p).split(sep).join('/');
}

function fileEntry(role, path, extra) {
  return { role, path: rel(path), bytes: statSync(path).size, sha256: sha256(path), ...extra };
}

// The system's archive tool (Windows bsdtar reads zip); each archive into its own new empty folder.
function extract(zip, dest) {
  if (existsSync(dest) && readdirSync(dest).length > 0) return;
  mkdirSync(dest, { recursive: true });
  const tar = process.platform === 'win32' ? 'C:\\Windows\\System32\\tar.exe' : 'bsdtar';
  execFileSync(tar, ['-xf', zip, '-C', dest], { stdio: 'inherit' });
}

function acgRole(name) {
  const n = name.toLowerCase();
  if (!/\.(jpg|jpeg|png)$/.test(n)) return null;
  if (n.includes('_color')) return 'albedo';
  if (n.includes('_normalgl')) return 'normal';
  if (n.includes('_roughness')) return 'roughness';
  if (n.includes('_ambientocclusion')) return 'ao';
  if (n.includes('_displacement')) return 'height';
  if (n.includes('_opacity')) return 'opacity';
  if (n.includes('_metalness')) return 'metalness';
  return null;
}

async function main() {
  mkdirSync(root, { recursive: true });
  const repo = resolve(here, '..', '..').toLowerCase();
  const r = root.toLowerCase();
  if (r === repo || r.startsWith(repo + sep)) {
    throw new Error(`refusing to download into the repository (${root})`);
  }
  const assets = [];
  let total = 0;
  // An asset already in the manifest whose every file is present with the
  // recorded size and SHA-256 is kept as recorded: no request is made for it.
  const previous = existsSync(manifestPath) ? JSON.parse(readFileSync(manifestPath, 'utf8')).assets || [] : [];
  const kept = (source, id, use) => {
    const a = previous.find((p) => p.source === source && p.id === id);
    if (!a || !a.files?.length) return null;
    for (const f of a.files) {
      const p = join(root, f.path);
      if (!existsSync(p) || statSync(p).size !== f.bytes || sha256(p) !== f.sha256) return null;
    }
    total += a.files.filter((f) => f.url).reduce((s, f) => s + f.bytes, 0);
    return { ...a, use };
  };
  for (const [id, use] of POLYHAVEN) {
    const k = kept('polyhaven', id, use);
    if (k) {
      assets.push(k);
      continue;
    }
    const info = await getJson(`https://api.polyhaven.com/info/${id}`);
    const files = await getJson(`https://api.polyhaven.com/files/${id}`);
    const pick = [
      ['albedo', files.Diffuse?.['2k']?.jpg],
      ['normal', files.nor_gl?.['2k']?.jpg],
      ['arm', files.arm?.['2k']?.jpg],
      ['height', files.Displacement?.['2k']?.jpg],
    ];
    const entries = [];
    for (const [role, f] of pick) {
      if (!f) throw new Error(`${id}: no 2k jpg ${role}`);
      const path = join(root, 'polyhaven', id, f.url.split('/').pop());
      if (total + f.size > BUDGET_BYTES) throw new Error(`stopping: ${id} would pass the ${BUDGET_BYTES} B download budget`);
      const got = await download(f.url, path, f.size);
      total += f.size;
      if (flag('--dry-run')) continue;
      entries.push(fileEntry(role, path, { url: f.url, resolution: '2k', md5: f.md5 }));
      if (got) console.log(`  ${rel(path)} ${(f.size / 1e6).toFixed(1)} MB`);
    }
    assets.push({
      source: 'polyhaven',
      id,
      page: `https://polyhaven.com/a/${id}`,
      name: info.name,
      authors: Object.keys(info.authors || {}),
      licence: 'CC0-1.0',
      size_m: (info.dimensions || []).map((v) => Math.round(v) / 1000),
      use,
      files: entries,
    });
    console.log(`polyhaven ${id}: ${Object.keys(info.authors || {}).join(', ')}`);
    await sleep(300);
  }
  for (const [id, res, use] of AMBIENTCG) {
    const k = kept('ambientcg', id, use);
    if (k) {
      assets.push(k);
      continue;
    }
    const j = await getJson(`https://ambientcg.com/api/v2/full_json?id=${id}&include=downloadData`);
    const a = (j.foundAssets || [])[0];
    if (!a) throw new Error(`${id}: not found`);
    const dl = a.downloadFolders?.default?.downloadFiletypeCategories?.zip?.downloads?.find((z) => z.attribute === res);
    if (!dl) throw new Error(`${id}: no ${res} zip`);
    const zip = join(root, 'ambientcg', id, dl.fileName);
    if (total + dl.size > BUDGET_BYTES) throw new Error(`stopping: ${id} would pass the ${BUDGET_BYTES} B download budget`);
    const got = await download(dl.downloadLink, zip, dl.size);
    total += dl.size;
    if (flag('--dry-run')) continue;
    if (got) console.log(`  ${rel(zip)} ${(dl.size / 1e6).toFixed(1)} MB`);
    const dest = join(root, 'ambientcg', id, 'x');
    extract(zip, dest);
    const entries = [fileEntry('archive', zip, { url: dl.downloadLink, resolution: res })];
    for (const f of readdirSync(dest).sort()) {
      const role = acgRole(f);
      if (role) entries.push(fileEntry(role, join(dest, f), { archive: rel(zip), resolution: res }));
    }
    assets.push({
      source: 'ambientcg',
      id,
      page: `https://ambientcg.com/a/${id}`,
      name: a.displayName,
      // ambientCG names no individual author on its assets; it asks to be credited as ambientCG.com.
      authors: ['ambientCG (ambientcg.com)'],
      licence: 'CC0-1.0',
      size_m: [],
      use,
      files: entries,
    });
    console.log(`ambientcg ${id}`);
    await sleep(500);
  }
  const downloaded = assets.flatMap((a) => a.files).filter((f) => f.url).reduce((s, f) => s + f.bytes, 0);
  // Every other file under the root: the sites' API listings and the preview
  // thumbnails used to choose sets (`_index/`), recorded so the folder holds
  // nothing unlisted. The game never reads them.
  const indexDir = join(root, '_index');
  const index = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else index.push({ path: rel(p), bytes: statSync(p).size, sha256: sha256(p) });
    }
  };
  if (existsSync(indexDir)) walk(indexDir);
  const indexBytes = index.reduce((s, f) => s + f.bytes, 0);
  const manifest = {
    schema: 'eo-cc0-manifest-v1',
    generatedBy: 'native/tools/fetch-cc0.mjs',
    licence: 'CC0-1.0 (https://creativecommons.org/publicdomain/zero/1.0/)',
    rootEnv: 'EO_CC0_ROOT',
    note: 'Downloads stay outside the repository; paths are relative to the root. The renderer verifies every file it reads against sha256 here.',
    downloadedBytes: downloaded,
    assets,
    indexNote: 'API listings (api.polyhaven.com, ambientcg.com/api/v2) and 256 px preview thumbnails (the APIs\' own preview links) fetched to choose sets; not read by the game.',
    indexBytes,
    index,
  };
  if (!flag('--dry-run')) {
    mkdirSync(dirname(manifestPath), { recursive: true });
    writeFileSync(manifestPath, JSON.stringify(manifest, null, 1) + '\n');
    console.log(`wrote ${manifestPath}: ${assets.length} assets, ${(downloaded / 1e6).toFixed(1)} MB downloaded`);
  } else {
    console.log(`dry run: ${(total / 1e6).toFixed(1)} MB would be downloaded`);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
