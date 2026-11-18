# Off-repo asset store

`artifacts/tripo`, `artifacts/vehicles` and `artifacts/characters` hold pipeline
**source and intermediates**: text-to-3D output for street props, Blender
scenes, texture plates. The game never loads any of it. What ships is the
derived copy under `public/assets/`, which the native build reads and
hash-verifies against the provenance file beside each asset.

Those sources are nearly 2 GB across a few hundred files. Committing them
would cost that much permanent Git history for bytes no build or
runtime reads, so the binaries live in an object store and only
`artifacts/ASSETS.lock.json` is committed. The JSON and Markdown provenance
beside them stays in Git.

This store is separate from the optional runtime content (people, motions,
scanned surfaces, sound, sign art, far-shore terrain), which is fetched from
its original source; see
[native/README.md](../native/README.md#optional-off-repository-assets).

## Layout

The store is **content-addressed**: each blob is written once at
`<remote>/sha256/<hash>`. Re-running a push is cheap, and byte-identical files
collapse to one upload.

`artifacts/ASSETS.lock.json` records `{ path, sha256, bytes }` per file. It is
the source of truth: pull verifies every download against it and refuses a
mismatch, and push refuses to upload a file that has drifted from it.

## Setup

Any [rclone](https://rclone.org) remote works; this is deliberately not tied
to a Git host. Install rclone with your system's package manager or from its
website, then configure a remote. For example, a Cloudflare R2 bucket
(any S3-compatible store works the same way):

```bash
rclone config create r2 s3 provider=Cloudflare \
  access_key_id=<KEY> secret_access_key=<SECRET> \
  endpoint=https://<ACCOUNT_ID>.r2.cloudflarestorage.com
export EO_ASSET_REMOTE=r2:<your-bucket>
```

For a local disk or network share, point at a path:

```bash
export EO_ASSET_REMOTE=/path/to/earth-online-assets
```

Put `EO_ASSET_REMOTE` and any credentials in your shell profile. They are
machine-specific and must not be committed. The project does not publish a
public remote; ask the maintainers if you need access to the pipeline
sources.

## Use

Run from the repository root (Node.js):

```bash
npm run assets:pull          # restore every missing or drifted binary
npm run assets:pull tripo    # restore a subset by path substring
npm run assets:lock          # after adding/replacing sources, rebuild the lock (from disk: drops unpulled entries)
npm run assets:lock:merge    # the same on a partial checkout: re-hashes what is present, keeps unpulled entries
npm run assets:lock:merge -- --drop artifacts/tripo/old-set/   # also drop kept entries under a path prefix
npm run assets:push          # upload blobs the remote does not have
npm run verify:asset-lock    # gate: nothing binary tracked, nothing drifted
```

`--drop <prefix>` (repeatable) works with `--merge`: it removes the kept
lock entries whose path starts with the prefix, for sources deleted from the
project. Without `--merge` the lock is rebuilt from disk alone and every
unpulled entry is dropped anyway, so use `--merge` on a partial checkout.

A fresh clone with no assets pulled is a **valid state**: the verifier checks
that what is present matches the lock, not that everything is present. Pull
only what a given task needs.

## Rules

- Never `git add` a `.glb`, `.blend`, `.fbx` or texture plate under those three
  trees. `.gitignore` blocks the known cases and `verify:asset-lock` fails if
  one slips through another path.
- `.blend1` (Blender autosave) and `.pyc` are regenerable junk: ignored,
  never archived.
- Adding a source asset: drop the file in, run `npm run assets:lock`, run
  `npm run assets:push`, then commit the lock change.
- Provenance beside an asset stays accurate. Regenerate it with the asset's
  build script; never hand-edit a digest.
