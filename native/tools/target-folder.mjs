// Where a fetch tool may write its downloads: exactly `<repo>/external/<set>`
// (git-ignored; the folder `scripts/setup-assets.py` fills and the game reads
// by default), or any folder outside the repository. Every other folder
// inside the repository is refused, so a download can never land among the
// tracked files. As in scripts/setup-assets.py, the target's links are
// resolved and each folder up its real path is compared with the repository
// and its external/ by identity (device and inode), so neither a different
// case on a case-insensitive disk nor a link into the repository gets past
// it; a part not made yet must be spelled exactly.
//
//   import { checkTarget } from './target-folder.mjs';
//   checkTarget(root, 'cc0');   // throws unless root is allowed

import { readlinkSync, realpathSync, statSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The repository root: two levels above this file (native/tools/). */
export const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** `<repo>/external/<set>`, the one target inside the repository. */
export const externalFolder = (set) => join(REPO, 'external', set);

/** Whether `a` and `b` both exist and are one folder (same device and inode). */
function same(a, b) {
  try {
    const x = statSync(a, { bigint: true });
    const y = statSync(b, { bigint: true });
    return x.dev === y.dev && x.ino === y.ino;
  } catch {
    return false;
  }
}

/** `p` with its links resolved: the real path of its deepest existing part, then the parts not made yet as written. */
function realPath(p, hops = 0) {
  const rest = [];
  for (let a = resolve(p); ; a = dirname(a)) {
    try {
      return join(realpathSync(a), ...rest);
    } catch {
      let link = null;
      try {
        link = readlinkSync(a);
      } catch {}
      // A link to a folder not made yet is followed too (a bounded number of times, for a loop).
      if (link !== null && hops < 40) return realPath(join(resolve(dirname(a), link), ...rest), hops + 1);
      if (dirname(a) === a) return resolve(p);
      rest.unshift(basename(a));
    }
  }
}

/** Throws unless `root` is `<repo>/external/<set>` or outside the repository. */
export function checkTarget(root, set) {
  const below = []; // the parts of the real path under the folder looked at
  for (let a = realPath(root); ; a = dirname(a)) {
    if (same(a, externalFolder(set))) {
      if (below.length === 0) return;
      break;
    }
    if (same(a, join(REPO, 'external'))) {
      if (below.length === 1 && below[0] === set) return;
      break;
    }
    if (same(a, REPO)) {
      if (below.length === 2 && below[0] === 'external' && below[1] === set) return;
      break;
    }
    if (dirname(a) === a) return; // outside the repository
    below.unshift(basename(a));
  }
  throw new Error(`refusing to download into the repository (${root}): inside it the only target is ${externalFolder(set)}`);
}
