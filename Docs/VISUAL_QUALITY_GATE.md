# Visual quality gate

**Independent audit date:** 2026-08-10
**Scope:** read-only visual assessment of the Earth Online capture set
available at that date. This is an internal quality gate. It is not a claim
of equivalence with any commercial product or technology.

The audit was made on the earlier browser build, which has since been
replaced by the native Rust build in `native/`. No review round has been
held on the native build, so the recorded verdict below still stands for
the project as a whole.

## Benchmark policy

The acceptance bar is "current-generation AAA open-world presentation". That bar
is defined in this repository only by the measurable rubric below.

Reviewers may consult external commercial reference material privately, on the
publisher's own official pages, to calibrate their eye. That material is
**out-of-repo by policy**:

- Never copy, commit, embed, train on, redistribute, or publish third-party
  screenshots or video frames in this repository or in any generated report.
- Never name a specific commercial title, publisher, or product in repository
  files, commit messages, code, docs, manifests, QA output, or issue text.
- Record only the rubric dimension, the scene category, the reviewer score, and
  the reviewer's written justification.

Reviewer notes and any calibration material stay outside the repository and
outside every served or published folder, and must never be committed.

## Style continuity during production

Keep reference identity, URLs and comparisons outside the repository, and
publish only neutral rubric judgments and the project's own evidence.

Record a visual-contract revision, shared lighting and settings, and approved
asset versions for the bounded reference street. Assess characters,
vegetation, buildings, vehicles and furnishing together under the same
lighting. An isolated asset approval or additional rendering capability does
not establish scene coherence. Inspect at normal camera scale and in motion;
reassess neighboring assets when shared materials, lighting, or detail
transitions change.

The reference street remains a candidate until independently accepted. Its
approval is an intermediate production milestone, not a whole-game quality
claim. All scene cards, reviewer counts, scores, rejection conditions, budgets,
and consecutive-round requirements below remain unchanged.

## Audit result: REJECT for the AAA-level claim

The audit inspected the city captures of 2026-08-09 (street, night, city,
canyon, hills and hero views) and the functional harness results recorded
with them. Those captures and the harness belonged to the browser build and
are no longer in the repository.

The functional harness reported **57 passed / 0 failed**. That is evidence
that the harness executed, city data loaded, and its defined checks passed;
it is not visual parity evidence. The inspected street frame had repeated
box buildings with uniform window grids, flat albedo, primitive vehicle
silhouettes, flat sidewalk/road construction, sparse stylized trees, and no
convincing human-scale material response. The inspected night frame had
uniformly dark building blocks and repeated emissive window dots, with no
believable indirect lighting, source shaping, streetscape depth, or
wet-surface response.

The result was therefore recorded as **procedural city prototype quality**,
not AAA-level realism. No individual dimension below passed an AAA-level
acceptance gate on the inspected street and night evidence.

**The verdict is REJECT.** It changes only through a new review round that
follows the protocol below. Test results, automated image scores and
single screenshots cannot change it.

## Why automated image scoring cannot approve realism

The earlier build scored frames from edge density, quantized color count and
mean luma, and compared them against a photo with global histograms,
perceptual hashes and LAB distance. Those tools have been removed with the
browser build, but the reasoning holds for any similar tool: such statistics
can be raised by repetitive windows, contrast or color grading, and they do
not evaluate geometric fidelity, PBR material behavior, animation, character
contact, traffic behavior or semantic correctness. They remain regression
signals only and cannot produce an AAA-level approval.

A scripted vote, or an A/B builder that pairs captures with stylized
references, is neither blind human evaluation nor an independent product
comparison.

## Required blind side-by-side protocol

For each review round, capture **the same eight scene cards** from Earth Online
at 16:9, 1440p or higher, 60 FPS target, with HUD/debug overlays off and a
recorded build hash/settings manifest. Pair each card with a scene *category*
brief, never with a copied image file:

1. pedestrian eye-level commercial street, clear day;
2. intersection/crosswalk with traffic and pedestrians;
3. dense building canyon at golden hour;
4. shoreline/waterfront at daylight;
5. wet street during drizzle or immediately after rain;
6. night street with shop, vehicle, and street lighting;
7. third-person character standing, walking, and stopping at a curb;
8. moving traversal clip (30 seconds) crossing a tile boundary.

In the native build, frames are captured offscreen with
`eo-app --bench --capture` (PNGs in `native/bench-out/`), with the time of
day, weather and switches given on the command line. There is no dedicated
scene-card capture tool yet; until one exists, record the exact command
line, commit hash and settings with every card.

An independent reviewer (critic) receives randomized A/B labels and the
common category brief, but no product name, implementation label, or prior
score. They must score the Earth Online card alone for production quality.
Use at least **five independent reviewers**. Reveal labels and aggregate only
after all forms are locked. A majority score is never enough: every critical
category must meet its per-card floor.

## 100-point scoring rubric

Score each item 0--5, multiply by its weight, and divide by 5. Use integer
scores only: 0 absent/broken, 1 placeholder, 2 prototype, 3 credible indie,
4 high-end production, 5 exceptional/current AAA bar.

| Dimension | Weight | What a 5 requires | Gate |
|---|---:|---|---:|
| Street and road realism | 18 | Real lane hierarchy, curbs, markings, drainage, intersections, correct scale, no planar/ribbon artifacts | >= 4.0 |
| Architecture and materials | 18 | Non-repeating facade language, correct construction depth, high-frequency detail, plausible PBR and weathering | >= 4.0 |
| Lighting and atmosphere | 14 | Directional sun/sky balance, local shadows, exposure control, believable night practicals and volumetrics | >= 4.0 |
| Water and weather | 10 | Wind/ripple/reflection response, shoreline contact, rain/wetness with physically legible roughness changes | >= 3.5 |
| Character grounding | 10 | Correct foot contact, slope/curb response, animation blend, shadow/contact, camera collision; no skating/clipping | >= 4.0 |
| NPC and traffic life | 10 | Diverse, scaled density; purposeful navigation, crossings, reaction, animation variety, no obvious loops | >= 3.5 |
| Composition and place identity | 8 | Readable SF context, landmark/terrain orientation, human-scale framing without debug-like repetition | >= 4.0 |
| Technical integrity | 12 | No visible z-fighting, LOD popping, seams, asset intersections, aliasing/shimmer, broken shadows, or frame hitches | >= 4.0 |

**Approval threshold:** weighted mean **>= 82/100**, no dimension below its
gate, no critical artifact in any card, and at least 4 of 5 reviewers score the
street, architecture, lighting, and character cards at >=4. A score of 70--81
is conditional; under 70 is reject. The AAA-level label additionally requires
two consecutive independent rounds to pass after a clean-build recapture.

## Critical automatic rejection conditions

- placeholder boxes, repeated facade grids, or primitive vehicles dominate a
  hero street frame;
- characters visibly float, skate, clip, or lack credible contact shadow;
- tile seams, road discontinuities, water gaps, z-fighting, or severe LOD pops;
- night scene is carried solely by uniformly emissive windows rather than local
  lighting and material response;
- a screenshot is cherry-picked, uses non-recorded settings, or omits the
  movement/tile-boundary verification clip;
- any review artifact names or embeds third-party commercial product material.

## Minimum next quality milestone

Do not spend the next visual pass on wider map coverage. Establish one
walkable, 150--250 m Ferry Building/Embarcadero hero corridor that passes all
eight cards: physically authored roads/sidewalks, varied facade kit with PBR,
high-quality civilian and vehicle assets, local lights/shadows, SF bay water,
and a grounded third-person character. Capture the mandatory set and run the
blind review before expanding to adjacent tiles.

## Evidence and remaining blockers

- No review round has been held on the native build.
- The native bench (`eo-app --bench`) measures frame time per camera path and
  can capture frames; it is timing and capture plumbing, not a quality
  evaluator.
- No trusted performance capture (frame-time percentile, resolution, GPU,
  active entity count) has accompanied a reviewed card set yet.
