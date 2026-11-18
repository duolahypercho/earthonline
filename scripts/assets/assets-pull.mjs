// Restore every locked binary that is missing or content-drifted locally.
// Each download is verified against the lock's sha256 before it is kept.
import { spawnSync } from 'node:child_process';
import { access, rename, rm, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { readLock, assetRemote, blobKey, sha256File, humanBytes, REPO_ROOT } from './asset-manifest.mjs';

const remote = assetRemote();
const lock = await readLock();
const only = process.argv.slice(2).filter((a) => !a.startsWith('-'));

let fetched = 0;
let ok = 0;
let bytes = 0;
for (const file of lock.files) {
  if (only.length && !only.some((p) => file.path.includes(p))) continue;
  const abs = path.join(REPO_ROOT, file.path);
  try {
    await access(abs);
    if (await sha256File(file.path) === file.sha256) { ok += 1; continue; }
    console.log(`!!  ${file.path} drifted from the lock, refetching`);
  } catch { /* missing - fetch it */ }

  await mkdir(path.dirname(abs), { recursive: true });
  const tmp = `${abs}.partial`;
  const res = spawnSync('rclone', ['copyto', `${remote}/${blobKey(file.sha256)}`, tmp], { stdio: 'inherit' });
  if (res.error) throw new Error('rclone not found. Install it (brew install rclone) - see Docs/ASSET-STORE.md.');
  if (res.status !== 0) { await rm(tmp, { force: true }); throw new Error(`rclone copyto failed for ${file.path}`); }

  const actual = await sha256File(path.relative(REPO_ROOT, tmp));
  if (actual !== file.sha256) {
    await rm(tmp, { force: true });
    throw new Error(`${file.path} downloaded as ${actual}, expected ${file.sha256}. Refusing a corrupt asset.`);
  }
  await rename(tmp, abs);
  fetched += 1;
  bytes += file.bytes;
  console.log(`dn  ${file.path}`);
}

console.log(`\nPulled ${fetched} files (${humanBytes(bytes)}), ${ok} already correct.`);
