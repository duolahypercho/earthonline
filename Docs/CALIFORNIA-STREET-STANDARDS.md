# California street standards

Every street, sidewalk, kerb, gutter, curb ramp, marking, crossing, signal,
sign, curb colour, parking and traffic-rule dimension or timing in the game
follows the California manuals. This is a binding project rule, not a style
preference.

## Sources, in order

| Rank | Source | Governs in San Francisco |
| --- | --- | --- |
| 1 | California MUTCD, 2026 Edition (Caltrans, in force 2026-01-18) | Every traffic control device on every public road: lines, crosswalks, limit lines, words, arrows, curb markings, signals, pedestrian signals, signs. Binding statewide (CVC 21400). |
| 2 | California Vehicle Code | Curb colours (21458), parking prohibitions (22500, 22500(n) daylighting, 22514 hydrants, 22522 curb ramps), parking position (22502), speed law (22348-22358.9). |
| 3 | Caltrans Highway Design Manual, 7th Edition | Geometry on the State Highway routes in SF (US-101 on Van Ness/Lombard/Richardson, SR-1 on 19th Ave/Park Presidio, SR-35 on Sloat/Skyline, SR-82, I-80, I-280). Bikeway criteria reach city streets through Streets and Highways Code 891. |
| 4 | Caltrans Standard Plans, 2025 Edition | Construction details on State routes (line patterns, word and crosswalk markings, curbs, curb ramps). |
| 5 | San Francisco local standards | City streets where the State is silent: Better Streets Plan sidewalk widths and zones, SF Public Works curb/gutter (87,170), curb ramp standard RX series, driveways, street lights, street acceptance minimums. Cannot override rank 1-2. |

A **surveyed measurement of the real street** (DataSF sidewalk widths,
right-of-way polygons, curb ramp census, hydrants, meters) beats a design
standard where it exists: the map is 1:1, and existing streets are not always
built to today's standard. The standard is then the fallback where no survey
exists and the bound a generated value must respect.

## Where the values live

- `scripts/world-data/sources/california-street-standards.json`: the cited
  table. Each entry has a value, unit, metric value, condition, document,
  edition, section and page, URL, applicability (`binding statewide`,
  `state highway only`, `local agency standard`, `guidance`) and confidence
  (`primary-source-checked`, `source-file-read-plot-unread` for a value taken
  from a sheet's source file by a person who has not seen the sheet plotted,
  `secondary`, `unverified`).
- `native/eo-data/src/standards.rs`: the Rust registry. `CaStandards::load`
  reads the table; code reads a value with `metric(id)`,
  `metric_key(id, key)` or `metric_range(id, key)`. An `unverified` entry
  has no metric value and returns an error; it can never become a silent
  default. The same file holds `CALIFORNIA_STREET_DEVIATIONS`, the recorded
  game deviations, read through `deviation(id)`.
- Code that needs a street value takes it from that registry or from survey
  data (`public/data/sf/`), and passes it down; `eo-street`, `eo-props`,
  `eo-sim` and `eo-player` follow this.

## The rule

1. A new or changed street, sidewalk, marking, signal, sign, parking or
   traffic-rule number comes from the registry or from survey data. A
   literal in that code is a defect.
2. A value that departs from the standard and from the survey is a **game
   deviation**. It goes in `CALIFORNIA_STREET_DEVIATIONS` with the standard
   id it departs from, the game value, the unit and the reason, at status
   `Pending`, and it is used only after the owner approves it
   (`Approved`, with `approved_by` naming a role and `approved_on` a date).
   `deviation()` returns an error for anything else. Readability, "looks
   better" and camera framing are reasons to ask, not approvals.
3. Changing an entry of the standards table needs a primary source: the
   document, edition, section or page and URL. Do not guess a number; mark it
   `unverified` and say what was found.
4. Do not loosen a test that checks these values. Loosening it is the same
   as weakening a gate and needs the owner's explicit approval
   ([AGENTS.md](../AGENTS.md#stop-and-ask)).

## Checking

The earlier browser build had a standalone checker script; it has been
removed with that build. Today the rule is enforced by review and by the
crate tests:

- `cargo test --release -p eo-data` loads the table and the deviation
  registry (`native/eo-data/tests/real_data.rs`,
  `native/eo-data/tests/ground_plan.rs`).
- `cargo test --release -p eo-street -p eo-props -p eo-sim` exercise the
  street surface, the placed furniture and parking, and the traffic rules,
  all built from the values they read from the registry.
- A reviewer checks that a patch adds no street dimension or timing literal
  and that any departure is in `CALIFORNIA_STREET_DEVIATIONS`.

A Rust checker that counts unsourced literals per file is a welcome
contribution.
