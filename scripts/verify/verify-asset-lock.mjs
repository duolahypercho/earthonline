// The `artifacts/` pipeline binaries live off-repo (Docs/ASSET-STORE.md).
// This asserts the two halves of that contract still hold:
//   1. no archived binary is tracked by git -- that is what bloated history;
//   2. every archived binary on disk is described by the committed lock, so a
//      fresh clone can restore exactly what the pipeline produced.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import {
  REPO_ROOT, LOCK_PATH, ROOTS, ARCHIVED_EXT, JUNK_EXT,
  walkArchived, readLock, sha256File, humanBytes,
} from '../assets/asset-manifest.mjs';

const tracked = execFileSync('git', ['ls-files', '-z'], { cwd: REPO_ROOT })
  .toString('utf8').split('\0').filter(Boolean);

const strayTracked = tracked.filter((file) => {
  if (!ROOTS.some((root) => file.startsWith(`${root}/`))) return false;
  const ext = path.extname(file).toLowerCase();
  return ARCHIVED_EXT.has(ext) || JUNK_EXT.has(ext);
});
assert.deepEqual(strayTracked, [],
  `binary pipeline sources must not be tracked; run \`git rm --cached\` on: ${strayTracked.join(', ')}`);

const lock = await readLock();
const locked = new Map(lock.files.map((f) => [f.path, f]));
const onDisk = await walkArchived();

const unlocked = onDisk.filter((p) => !locked.has(p));
assert.deepEqual(unlocked, [],
  `archived binaries missing from ${LOCK_PATH}; run \`npm run assets:lock\`: ${unlocked.join(', ')}`);

// Only hash what is actually present: a clone that has not run `assets:pull`
// is a valid state, so absence is not a failure here. Drift is.
let verified = 0;
let bytes = 0;
for (const rel of onDisk) {
  const entry = locked.get(rel);
  const actual = await sha256File(rel);
  assert.equal(actual, entry.sha256, `${rel} drifted from the lock (${actual} != ${entry.sha256})`);
  verified += 1;
  bytes += entry.bytes;
}

const distinct = new Set(lock.files.map((f) => f.sha256));
console.log(JSON.stringify({
  result: 'asset lock passed',
  lockedFiles: lock.files.length,
  distinctBlobs: distinct.size,
  presentLocally: verified,
  presentBytes: humanBytes(bytes),
}, null, 2));
