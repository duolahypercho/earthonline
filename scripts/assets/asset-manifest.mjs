// Shared helpers for the off-repo binary asset store.
//
// `artifacts/` holds pipeline SOURCE and intermediates: Tripo/Blender output
// that the game never loads. The shipped, hash-verified copies live under
// `public/assets/`. Keeping the sources in git cost 900+ MB of history, so the
// binaries now live in an object store and only this lock file is committed.
//
// The store is content-addressed: a blob is written once per distinct sha256,
// so the byte-identical copies the Tripo CLI leaves behind (`ansem-*.glb` and
// its `tripo-out/*/model.glb` twin) collapse to a single upload.
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { readFile, writeFile, readdir, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
export const LOCK_PATH = 'artifacts/ASSETS.lock.json';

// Trees whose heavy files are stored off-repo. Their JSON/markdown provenance
// stays committed alongside artifacts/ASSETS.lock.json.
export const ROOTS = ['artifacts/tripo', 'artifacts/vehicles', 'artifacts/ocean'];

// Archived to the store and recorded in the lock.
export const ARCHIVED_EXT = new Set(['.glb', '.blend', '.fbx', '.png', '.jpg', '.jpeg']);

// Regenerable local junk: never committed, never archived. `.blend1` is
// Blender's autosave backup, `.pyc` is a bytecode cache.
export const JUNK_EXT = new Set(['.blend1', '.pyc']);

export async function walkArchived(roots = ROOTS) {
  const found = [];
  for (const root of roots) {
    const abs = path.join(REPO_ROOT, root);
    let entries;
    try {
      entries = await readdir(abs, { withFileTypes: true, recursive: true });
    } catch (err) {
      if (err.code === 'ENOENT') continue;
      throw err;
    }
    for (const entry of entries) {
      if (!entry.isFile()) continue;
      if (!ARCHIVED_EXT.has(path.extname(entry.name).toLowerCase())) continue;
      const full = path.join(entry.parentPath ?? entry.path, entry.name);
      found.push(path.relative(REPO_ROOT, full));
    }
  }
  return found.sort();
}

export function sha256File(relPath) {
  return new Promise((resolve, reject) => {
    const hash = createHash('sha256');
    const stream = createReadStream(path.join(REPO_ROOT, relPath));
    stream.on('error', reject);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolve(hash.digest('hex')));
  });
}

export async function fileSize(relPath) {
  return (await stat(path.join(REPO_ROOT, relPath))).size;
}

export async function readLock() {
  const raw = await readFile(path.join(REPO_ROOT, LOCK_PATH), 'utf8');
  const lock = JSON.parse(raw);
  if (!Array.isArray(lock.files)) throw new Error(`${LOCK_PATH} has no files[]`);
  return lock;
}

export async function writeLock(files) {
  const lock = {
    version: 1,
    note: 'Binary pipeline sources stored off-repo. Run `npm run assets:pull` to fetch. See Docs/ASSET-STORE.md.',
    files: files.slice().sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0)),
  };
  await writeFile(path.join(REPO_ROOT, LOCK_PATH), `${JSON.stringify(lock, null, 2)}\n`);
  return lock;
}

// `rclone` remote root, e.g. `r2:<your-bucket>` or a plain directory
// like `/path/to/eo-assets` for a local/NAS store.
export function assetRemote() {
  const remote = process.env.EO_ASSET_REMOTE;
  if (!remote) {
    throw new Error('EO_ASSET_REMOTE is unset. Set it to an rclone remote (e.g. "r2:<your-bucket>") or a local directory. See Docs/ASSET-STORE.md.');
  }
  return remote.replace(/\/+$/, '');
}

export function blobKey(sha256) {
  return `sha256/${sha256}`;
}

export function humanBytes(n) {
  const units = ['B', 'KB', 'MB', 'GB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i += 1; }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}
