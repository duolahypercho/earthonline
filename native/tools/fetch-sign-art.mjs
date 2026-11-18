#!/usr/bin/env node
// download the official sign artwork and the sign font the native sign
// atlases read, into `<repo>/external/sign-art` (git-ignored) or a folder
// outside the repository (any other folder inside it is refused,
// target-folder.mjs), and write the `files`
// list of the provenance manifest `native/eo-props/data/sign-art-manifest.json`
// (per file: source URL and the ZIP entry it came from, licence or
// public-domain basis, retrieval date, bytes, SHA-256). The manifest's other
// sections (palette, layouts, face table, calibration) are hand-written and
// kept as they are.
//
//   node native/tools/fetch-sign-art.mjs --root <folder> [--dry-run]   # or EO_SIGN_ART_ROOT
//
// Sources, and nothing else:
//   * FHWA Standard Highway Signs 2024 (MUTCD 11th Edition) vector layouts,
//     https://mutcd.fhwa.dot.gov/kno-shs_2024-release-status/index.htm. Each
//     release is one large ZIP; this reads the ZIP's directory and the wanted
//     SVG entries with HTTP Range requests instead of downloading it whole.
//     Public domain (a work of the U.S. Government, 17 U.S.C. 105).
//   * Caltrans California sign specification sheets (PDF),
//     https://dot.ca.gov/programs/safety-programs/sign-specs. Public domain per
//     https://dot.ca.gov/conditions-of-use. Read by a person to transcribe the
//     layouts; the game never parses them, it only checks their digest.
//   * Overpass 3.0.5 (Red Hat, SIL OFL-1.1), the GitHub release ZIP of
//     https://github.com/RedHatOfficial/Overpass, extracted with the system's
//     archive tool into its own new folder.
// A generic User-Agent, nothing about the user or the project in any request,
// no account, one request at a time. Nothing downloaded is executed. Files
// already present are not fetched again.

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { inflateRawSync } from 'node:zlib';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { checkTarget } from './target-folder.mjs';

const UA = 'sign-art-fetch/1.0';
const here = dirname(fileURLToPath(import.meta.url));
const manifestPath = resolve(here, '..', 'eo-props', 'data', 'sign-art-manifest.json');
const argv = process.argv.slice(2);
const flag = (k) => argv.includes(k);
const opt = (k, d) => {
  const i = argv.indexOf(k);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : d;
};
const root = resolve(opt('--root', process.env.EO_SIGN_ART_ROOT) || (() => { console.error('usage: fetch-sign-art.mjs --root <folder> (or set EO_SIGN_ART_ROOT)'); process.exit(2); })());
const ALLOWED = new Set(['mutcd.fhwa.dot.gov', 'dot.ca.gov', 'github.com', 'objects.githubusercontent.com', 'release-assets.githubusercontent.com', 'codeload.github.com']);
const TODAY = new Date().toLocaleDateString('sv'); // local YYYY-MM-DD

const FHWA = 'https://mutcd.fhwa.dot.gov/kno-shs_2024-release-status/zip_files/';
const PD_US = 'Public domain: a work of the U.S. Government (17 U.S.C. 105)';
const PD_CA = 'Public domain per the Caltrans Conditions of Use ("information presented on this website, unless otherwise indicated, is considered in the public domain")';
const OFL = 'SIL Open Font License 1.1 (Overpass is offered under OFL-1.1 or LGPL-2.1; used under OFL-1.1)';
const R2 = '2024_SHS_Release_2-New_Regulatory_TTC_School_Signs_Vector_Graphics.zip';
const R3A = '2024_SHS_Release_3-Regulatory_Warning_TTC_School_Signs_Vector_Graphics_1-of-2.zip';
const R4 = '2024_SHS_Release_4-Regulatory_Warning_TTC_School_Signs_Vector_Graphics.zip';
const R6 = '2024_SHS_Release_6-Signs_Vector_Graphics.zip';
const REL = { [R2]: 'Release 2', [R3A]: 'Release 3 (file 1 of 2)', [R4]: 'Release 4', [R6]: 'Release 6' };

// [id, release zip, entry name (exact), size in inches]
const SVGS = [
  ['R1-1', R3A, 'R01-01 Stop/R01-01 Stop 30x30.svg', [30, 30]],
  ['R1-3P', R3A, 'R01-03P All Way (plaque)/R01-03P All-Way (plaque) 30x12.svg', [30, 12]],
  ['R2-1', R3A, 'R02-01 Speed Limit/R02-01 Speed Limit 24x30.svg', [24, 30]],
  ['R3-1', R3A, 'R03-01 Movement Prohibition - No Right Turn/R03-01 Movement Prohibition - No Right Turn 36x36.svg', [36, 36]],
  ['R3-2', R3A, 'R03-02 Movement Prohibition - No Left Turn/R03-02 Movement Prohibition - No Left Turn 36x36.svg', [36, 36]],
  ['R3-7R', R2, 'R03-07 Right (Left) Lane Must Turn Right (Left)/R03-07R Right Lane Must Turn Right 30x30.svg', [30, 30]],
  ['R3-11e', R2, 'R03-11e Preferential Lane Operation - Full-Time (Right Lane Buses Only) (post-mounted)/R03-11e Preferential Lane Operation - Full-Time (Right Lane Buses Only) (post-mounted) 30x42.svg', [30, 42]],
  ['R5-1', R3A, 'R05-01 Do Not Enter/R05-01 Do Not Enter 30x30.svg', [30, 30]],
  ['R6-1R', R4, 'R06-01 One Way/R06-01R One Way 36x12.svg', [36, 12]],
  ['R6-1L', R4, 'R06-01 One Way/R06-01L One Way 36x12.svg', [36, 12]],
  ['R7-8', R3A, 'R07-08 Reserved Parking - Accessible/R07-08 Reserved Parking - Accessible 12x18.svg', [12, 18]],
  ['R7-20', R3A, 'R07-20 Parking Fee Station, Multispace Meter/R07-20 Parking Fee Station, Multispace Meter 24x18.svg', [24, 18]],
  ['R8-3', R3A, 'R08-03 No Parking (symbol)/R08-03 No Parking (symbol) 24x24.svg', [24, 24]],
  ['S1-1', R3A, 'S01-01 School/S01-01 School 30x30.svg', [30, 30]],
  ['W8-1', R3A, 'W08-01 Bump/W08-01 Bump 30x30.svg', [30, 30]],
  ['W11-2', R3A, 'W11-02 Pedestrian/W11-02 Pedestrian 30x30.svg', [30, 30]],
  ['W16-7PL', R3A, 'W16-07P Downward Diagonal Arrow (plaque)/W16-07PL Downward Diagonal Arrow (plaque) 12x6.svg', [12, 6]],
  ['W16-9P', R3A, 'W16-09P Ahead (plaque)/W16-09P Ahead (plaque) 24x12.svg', [24, 12]],
  ['W17-1', R3A, 'W17-01 Speed Hump/W17-01 Speed Hump 30x30.svg', [30, 30]],
  ['W20-1', R4, 'W20-01 Road (Street) Work (with distance)/W20-01 Road Work (with distance) Ahead 30x30.svg', [30, 30]],
  ['D3-1a', R6, 'SHS Release 6/D03-01a Street Name with Route Sign (1 line)/D03-01a Street Name with Route Sign (1 line) VARx8.svg', null],
  ['D11-1', R6, 'SHS Release 6/D11-01 Bike Route/D11-01 Bike Route 24x18.svg', [24, 18]],
];

const CT = 'https://dot.ca.gov/-/media/dot-media/programs/safety-programs/documents/signs/r/';
// [id, file, what]
const PDFS = [
  ['R26(CA)', 'f0019008-r26-a11y.pdf', 'NO PARKING ANY TIME (3/22/91)'],
  ['R30B(CA)', 'r30b-ca-a11y.pdf', 'No Parking 10AM TO 12PM WEDNESDAY STREET SWEEPING (3/1/24)'],
  ['R32(CA)', 'f0019038-r32-a11y.pdf', 'Limited Hour Parking Specific Hours (3/22/91)'],
  ['R32C(CA)', 'r32c-a11y.pdf', '2 HOUR PARKING 8AM TO 6PM DISTRICT 7 PERMITS EXEMPT (2/17/09)'],
  ['R39-1(CA)', 'f0019057-r39-1-a11y.pdf', 'NO DOUBLE PARKING ANYTIME COMMERCIAL VEHICLES INCLUDED (1/21/10)'],
  ['R99(CA)', 'r99-r99b-a11y.pdf', 'Accessible Parking Only (7/1/08)'],
];

const OVERPASS_ZIP = 'https://github.com/RedHatOfficial/Overpass/releases/download/v3.0.5/overpass-3.0.5.zip';
// [id, path inside the release ZIP]
const FONTS = [
  ['overpass-regular', 'Overpass-3.0.5/webfonts/overpass-webfont/overpass-regular.ttf'],
  ['overpass-semibold', 'Overpass-3.0.5/webfonts/overpass-webfont/overpass-semibold.ttf'],
  ['overpass-bold', 'Overpass-3.0.5/webfonts/overpass-webfont/overpass-bold.ttf'],
  ['overpass-licence', 'Overpass-3.0.5/LICENSE.md'],
];

function checkHost(url) {
  const h = new URL(url).hostname;
  if (!ALLOWED.has(h)) throw new Error(`refusing a request to ${h}`);
}

async function get(url, headers = {}) {
  checkHost(url);
  const r = await fetch(url, { headers: { 'User-Agent': UA, ...headers }, redirect: 'follow' });
  checkHost(r.url);
  return r;
}

async function range(url, a, b) {
  const r = await get(url, { Range: `bytes=${a}-${b}` });
  if (r.status !== 206) throw new Error(`${url}: no range support (HTTP ${r.status})`);
  return { buf: Buffer.from(await r.arrayBuffer()), total: Number((r.headers.get('content-range') || '').split('/')[1]) };
}

// The central directory of a remote ZIP (ZIP64 aware).
const dirs = new Map();
async function zipDir(url) {
  if (dirs.has(url)) return dirs.get(url);
  const { total: size } = await range(url, 0, 0);
  const tailLen = Math.min(size, 1 << 16);
  const { buf: tail } = await range(url, size - tailLen, size - 1);
  let eocd = -1;
  for (let i = tail.length - 22; i >= 0; i--) if (tail.readUInt32LE(i) === 0x06054b50) { eocd = i; break; }
  if (eocd < 0) throw new Error(`${url}: no end of central directory`);
  let cdSize = tail.readUInt32LE(eocd + 12), cdOff = tail.readUInt32LE(eocd + 16);
  if (cdOff === 0xffffffff) {
    const z64 = Number(tail.readBigUInt64LE(eocd - 20 + 8));
    const { buf: z } = await range(url, z64, z64 + 55);
    cdSize = Number(z.readBigUInt64LE(40)); cdOff = Number(z.readBigUInt64LE(48));
  }
  const { buf: cd } = await range(url, cdOff, cdOff + cdSize - 1);
  const entries = new Map();
  for (let p = 0; p < cd.length && cd.readUInt32LE(p) === 0x02014b50;) {
    const method = cd.readUInt16LE(p + 10);
    let csize = cd.readUInt32LE(p + 20), usize = cd.readUInt32LE(p + 24), off = cd.readUInt32LE(p + 42);
    const nl = cd.readUInt16LE(p + 28), el = cd.readUInt16LE(p + 30), kl = cd.readUInt16LE(p + 32);
    const name = cd.slice(p + 46, p + 46 + nl).toString('utf8');
    for (let e = p + 46 + nl; e < p + 46 + nl + el;) {
      const id = cd.readUInt16LE(e), sz = cd.readUInt16LE(e + 2);
      if (id === 1) {
        let q = e + 4;
        if (usize === 0xffffffff) { usize = Number(cd.readBigUInt64LE(q)); q += 8; }
        if (csize === 0xffffffff) { csize = Number(cd.readBigUInt64LE(q)); q += 8; }
        if (off === 0xffffffff) off = Number(cd.readBigUInt64LE(q));
      }
      e += 4 + sz;
    }
    entries.set(name, { method, csize, usize, off });
    p += 46 + nl + el + kl;
  }
  dirs.set(url, entries);
  return entries;
}

async function zipEntry(url, name, out) {
  if (existsSync(out)) return false;
  const e = (await zipDir(url)).get(name);
  if (!e) throw new Error(`${url}: no entry ${name}`);
  if (flag('--dry-run')) return false;
  const { buf } = await range(url, e.off, e.off + 30 + 1024 + e.csize);
  if (buf.readUInt32LE(0) !== 0x04034b50) throw new Error(`${name}: bad local header`);
  const start = 30 + buf.readUInt16LE(26) + buf.readUInt16LE(28);
  const data = buf.slice(start, start + e.csize);
  const raw = e.method === 8 ? inflateRawSync(data) : data;
  if (raw.length !== e.usize) throw new Error(`${name}: ${raw.length} B, the directory says ${e.usize}`);
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, raw);
  return true;
}

async function download(url, out) {
  if (existsSync(out)) return false;
  if (flag('--dry-run')) return false;
  const r = await get(url);
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, Buffer.from(await r.arrayBuffer()));
  return true;
}

const sha256 = (p) => createHash('sha256').update(readFileSync(p)).digest('hex');
const rel = (p) => relative(root, p).split(sep).join('/');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  checkTarget(root, 'sign-art');
  mkdirSync(root, { recursive: true });
  const old = existsSync(manifestPath) ? JSON.parse(readFileSync(manifestPath, 'utf8')) : {};
  const prev = new Map((old.files || []).map((f) => [f.id, f]));
  const files = [];
  const entry = (id, path, extra) => {
    const p = prev.get(id);
    const sha = sha256(path);
    // Keep the first retrieval date of an unchanged file.
    const retrieved = p && p.sha256 === sha && p.retrieved ? p.retrieved : TODAY;
    files.push({ id, path: rel(path), bytes: statSync(path).size, sha256: sha, retrieved, ...extra });
  };
  for (const [id, zip, name, size] of SVGS) {
    const base = name.split('/').pop();
    const out = join(root, 'fhwa', 'shs2024', base.split(' ')[0], base);
    if (await zipEntry(FHWA + zip, name, out)) console.log(`  ${rel(out)}`);
    if (!flag('--dry-run')) entry(id, out, { source: `FHWA Standard Highway Signs 2024 (MUTCD 11th Ed.), ${REL[zip]} vector graphics`, url: FHWA + zip, zipEntry: name, licence: PD_US, licenceUrl: 'https://www.copyright.gov/title17/92chap1.html#105', ...(size ? { sizeIn: size } : {}) });
    await sleep(200);
  }
  for (const [id, file, what] of PDFS) {
    const out = join(root, 'caltrans', 'r', file);
    if (await download(CT + file, out)) console.log(`  ${rel(out)}`);
    if (!flag('--dry-run')) entry(id, out, { source: `Caltrans California Sign Specification sheet ${id}: ${what}`, url: CT + file, licence: PD_CA, licenceUrl: 'https://dot.ca.gov/conditions-of-use' });
    await sleep(300);
  }
  const zip = join(root, 'overpass', 'overpass-3.0.5.zip');
  if (await download(OVERPASS_ZIP, zip)) console.log(`  ${rel(zip)}`);
  const x = join(root, 'overpass', 'x');
  if (!flag('--dry-run')) {
    const missing = FONTS.filter(([, p]) => !existsSync(join(x, p)));
    if (missing.length) {
      mkdirSync(x, { recursive: true });
      const tar = process.platform === 'win32' ? 'C:\\Windows\\System32\\tar.exe' : 'bsdtar';
      execFileSync(tar, ['-xf', zip, '-C', x, ...missing.map(([, p]) => p)], { stdio: 'inherit' });
    }
    entry('overpass-release-zip', zip, { source: 'Overpass 3.0.5 release archive (Red Hat)', url: OVERPASS_ZIP, licence: OFL, licenceUrl: 'https://github.com/RedHatOfficial/Overpass/blob/master/LICENSE.md' });
    for (const [id, p] of FONTS) entry(id, join(x, p), { source: 'Overpass 3.0.5 (Red Hat)', url: OVERPASS_ZIP, archiveEntry: p, licence: OFL, licenceUrl: 'https://github.com/RedHatOfficial/Overpass/blob/master/LICENSE.md' });
  }
  if (flag('--dry-run')) { console.log('dry run: nothing written'); return; }
  const manifest = { ...old, schema: 'eo-sign-art-manifest-v1', generatedBy: 'native/tools/fetch-sign-art.mjs', rootEnv: 'EO_SIGN_ART_ROOT', downloadedBytes: files.filter((f) => !f.archiveEntry).reduce((s, f) => s + f.bytes, 0), files };
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 1).replace(/\n/g, '\r\n') + '\r\n');
  console.log(`wrote ${manifestPath}: ${files.length} files`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
