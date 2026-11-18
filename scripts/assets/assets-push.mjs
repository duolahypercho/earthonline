// Upload every locked blob that the remote does not already hold.
// Content-addressed, so re-running is cheap and duplicate files upload once.
import { spawnSync } from 'node:child_process';
import { readLock, assetRemote, blobKey, sha256File, humanBytes, REPO_ROOT } from './asset-manifest.mjs';
import path from 'node:path';

const remote = assetRemote();
const lock = await readLock();

const listed = spawnSync('rclone', ['lsf', `${remote}/sha256/`], { encoding: 'utf8' });
if (listed.error) throw new Error('rclone not found. Install it (brew install rclone) - see Docs/ASSET-STORE.md.');
// A remote with no blobs yet lists nothing; a genuine failure still reports.
const present = new Set(
  (listed.status === 0 ? listed.stdout : '').split('\n').map((l) => l.trim()).filter(Boolean));
if (listed.status !== 0 && !/directory not found/i.test(listed.stderr || '')) {
  throw new Error(`rclone lsf failed: ${(listed.stderr || '').trim()}`);
}

const blobs = new Map();
for (const f of lock.files) if (!blobs.has(f.sha256)) blobs.set(f.sha256, f);

let uploaded = 0;
let skipped = 0;
let bytes = 0;
for (const [sha256, file] of blobs) {
  if (present.has(sha256)) { skipped += 1; continue; }
  // Never upload a file whose content drifted from the lock.
  const actual = await sha256File(file.path);
  if (actual !== sha256) {
    throw new Error(`${file.path} is ${actual} but the lock says ${sha256}. Run \`npm run assets:lock\` first.`);
  }
  const dest = `${remote}/${blobKey(sha256)}`;
  const res = spawnSync('rclone', ['copyto', path.join(REPO_ROOT, file.path), dest], { stdio: 'inherit' });
  if (res.status !== 0) throw new Error(`rclone copyto failed for ${file.path}`);
  uploaded += 1;
  bytes += file.bytes;
  console.log(`up  ${file.path} -> ${blobKey(sha256).slice(0, 18)}...`);
}

console.log(`\nPushed ${uploaded} blobs (${humanBytes(bytes)}), ${skipped} already on the remote.`);
