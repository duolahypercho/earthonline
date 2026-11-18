// Regenerate `artifacts/ASSETS.lock.json` from the binaries currently on disk.
// Run after adding or replacing pipeline source assets, then `npm run assets:push`.
//
// `--merge` keeps every locked entry whose file is not on this checkout (a
// clone with nothing pulled is a valid state, Docs/ASSET-STORE.md) and
// re-hashes only what is present. Without it the lock is rebuilt from disk
// alone, which DROPS every unpulled asset: on a worktree holding 47 of 330
// locked files it removed 298 entries. With `--merge`, `--drop <prefix>`
// (repeatable) also removes the kept entries under a path prefix, for
// sources deleted from the project.
import { walkArchived, sha256File, fileSize, readLock, writeLock, humanBytes } from './asset-manifest.mjs';

const merge = process.argv.includes('--merge');
const drop = process.argv.flatMap((a, i, all) => (a === '--drop' && all[i + 1] ? [all[i + 1]] : []));
const paths = await walkArchived();
const files = [];
let bytes = 0;
for (const rel of paths) {
  const [sha256, size] = await Promise.all([sha256File(rel), fileSize(rel)]);
  files.push({ path: rel, sha256, bytes: size });
  bytes += size;
}
let kept = 0;
if (merge) {
  const present = new Set(paths);
  for (const entry of (await readLock()).files) {
    if (present.has(entry.path)) continue;
    if (drop.some((prefix) => entry.path.startsWith(prefix))) continue;
    files.push(entry);
    kept += 1;
  }
}

const lock = await writeLock(files);
const unique = new Set(files.map((f) => f.sha256));
const uniqueBytes = [...new Map(files.map((f) => [f.sha256, f.bytes])).values()]
  .reduce((a, b) => a + b, 0);
const lockedBytes = files.reduce((a, f) => a + f.bytes, 0);

console.log(`Locked ${lock.files.length} files, ${humanBytes(bytes)} on disk${merge ? `, ${kept} unpulled entries kept` : ''}.`);
console.log(`${unique.size} distinct blobs, ${humanBytes(uniqueBytes)} to store (dedup saves ${humanBytes(lockedBytes - uniqueBytes)}).`);
