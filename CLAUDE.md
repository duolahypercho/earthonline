# CLAUDE.md

Rules for coding agents in this repository. The full guide is
[AGENTS.md](AGENTS.md): read it first. This file is a short summary; where
the two differ, AGENTS.md wins.

## Essentials

- **Layout.** The game is the Rust workspace in `native/`
  ([native/README.md](native/README.md)). City data is in `public/data/sf/`,
  shipped models and textures in `public/assets/`, asset provenance and the
  asset-store lock in `artifacts/`.
- **Build and test** from `native/`: `cargo build --release`,
  `cargo test --release -p <crate>` for every crate you touch, and
  `./target/release/eo-app --bench --check-max` for anything that can cost
  frame time. Run the matching rows of the verification matrix in
  [AGENTS.md](AGENTS.md#verification-matrix).
- **Commits.** `type(scope): summary`, one subsystem per commit,
  `git diff --check` clean, only your own files staged, no local scratch or
  build output, no binary under `artifacts/`. Never force-push.
- **Provenance.** Every data file and asset has a recorded source, licence
  and digest. Regenerate manifests with their scripts; never hand-edit a
  digest. Only `artifacts/ASSETS.lock.json` and provenance are tracked under
  `artifacts/`; run `npm run verify:asset-lock` when anything there changes.
  See [Docs/ASSET-STORE.md](Docs/ASSET-STORE.md).
- **Street standards.** Street, sidewalk, kerb, marking, crossing, signal,
  sign, curb colour, parking and traffic-rule values come from
  `scripts/world-data/sources/california-street-standards.json` through
  `native/eo-data/src/standards.rs`, or from survey data. No literals. A
  departure is a deviation the owner approves, recorded in
  `CALIFORNIA_STREET_DEVIATIONS`. See
  [Docs/CALIFORNIA-STREET-STANDARDS.md](Docs/CALIFORNIA-STREET-STANDARDS.md).
- **Quality claims.** No visual claim from functional checks; before/after
  means two matched captures; never weaken a failing gate. The visual gate's
  recorded verdict is REJECT, and only a new blind review under
  [Docs/VISUAL_QUALITY_GATE.md](Docs/VISUAL_QUALITY_GATE.md) can change it.
- **Frame budget.** The whole frame stays under 10 ms. State the cost of a
  change per bench path.
- **Kill switches.** Every visual feature has a `--no-<feature>` switch that
  restores the previous output; a new feature adds one.
- **Paths.** Never hard-code an absolute path; off-repository content is
  found through environment variables.
- **Neutral text.** Never name a commercial game, studio or publisher in any
  tracked file, commit or issue.
