### Frame time on the bench paths: no measurable change

Measured on the owner's Mac (Apple silicon, macOS 27.2), release build, shared with other sessions.

| Metric | Before (median) | After (median) | Change | Verdict | Runs | Range before → after |
|---|---:|---:|---:|---|---:|---|
| street_walk frame time (median) | 4.91 ms | 4.76 ms | -3.0% | within noise | 5 / 5 | 4.76–5.08 → 4.75–4.89 |
| elevated_sweep frame time (median) | 6.26 ms | 6.32 ms | +1.0% | within noise | 5 / 5 | 6.19–6.44 → 6.29–6.50 |
| city_overview frame time (median) | 8.54 ms | 8.57 ms | +0.3% | within noise | 5 / 5 | 8.45–8.80 → 8.54–8.81 |
| night_kit_street frame time (median) | 5.09 ms | 5.11 ms | +0.5% | within noise | 5 / 5 | 5.05–5.45 → 5.05–5.29 |
| night_waterfront frame time (median) | 6.39 ms | 6.41 ms | +0.4% | within noise | 5 / 5 | 6.38–6.67 → 6.35–6.55 |

How each number was measured:

- street_walk frame time (median): `eo-app --bench --segments street_walk,elevated_sweep,city_overview,night_kit_street,night_waterfront (default quality, external assets set; main and branch alternated, 5 runs each)`
- elevated_sweep frame time (median): `eo-app --bench --segments street_walk,elevated_sweep,city_overview,night_kit_street,night_waterfront (default quality, external assets set; main and branch alternated, 5 runs each)`
- city_overview frame time (median): `eo-app --bench --segments street_walk,elevated_sweep,city_overview,night_kit_street,night_waterfront (default quality, external assets set; main and branch alternated, 5 runs each)`
- night_kit_street frame time (median): `eo-app --bench --segments street_walk,elevated_sweep,city_overview,night_kit_street,night_waterfront (default quality, external assets set; main and branch alternated, 5 runs each)`
- night_waterfront frame time (median): `eo-app --bench --segments street_walk,elevated_sweep,city_overview,night_kit_street,night_waterfront (default quality, external assets set; main and branch alternated, 5 runs each)`
