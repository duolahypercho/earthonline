// Where a fetch tool may write its downloads: exactly `<repo>/external/<set>`
// (git-ignored; the folder `scripts/setup-assets.py` fills and the game reads
// by default), or any folder outside the repository. Every other folder
// inside the repository is refused, so a download can never land among the
// tracked files. Paths are compared without regard to case (Windows and
// macOS file systems), which only ever refuses more.
//
//   import { checkTarget } from './target-folder.mjs';
//   checkTarget(root, 'cc0');   // throws unless root is allowed

import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The repository root: two levels above this file (native/tools/). */
export const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** `<repo>/external/<set>`, the one target inside the repository. */
export const externalFolder = (set) => join(REPO, 'external', set);

/** Throws unless `root` is `<repo>/external/<set>` or outside the repository. */
export function checkTarget(root, set) {
  const r = resolve(root).toLowerCase();
  const repo = REPO.toLowerCase();
  if (r === externalFolder(set).toLowerCase()) return;
  if (r === repo || r.startsWith(repo + sep)) {
    throw new Error(`refusing to download into the repository (${root}): inside it the only target is ${externalFolder(set)}`);
  }
}
