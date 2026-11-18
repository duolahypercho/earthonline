# Audio provenance

Every audio file under `artifacts/audio/` must be listed here before it is
referenced by a manifest. Rules:

1. A file is either **original** (recorded or synthesised by a named
   contributor for this project) or **CC0** (public-domain dedication, with
   the source noted). Nothing else is accepted: no CC-BY, no "royalty-free"
   packs with terms, no game or film audio, no samples of third-party
   media.
2. Original recordings: performers consent to unrestricted use in this
   project; note that here.
3. CC0 sources: record the source name, the item title or id, and the
   retrieval date. Do not vendor the source's own metadata files.
4. Per `CLAUDE.md`, never name a commercial game, studio, or publisher in
   this file, in filenames, or in metadata tags. Strip tags on encode
   (`-map_metadata -1`).
5. Modified CC0 files (trimmed, pitched, layered) stay CC0; still list the
   source.

## Sessions

### Session template (copy per recording session)

- **Date:**
- **Contributors / performers:** (initials are fine)
- **Room / mic:**
- **Files produced:** glob or list, e.g. `barks/a-*.ogg`, `barks/b-*.ogg`
- **Licence:** original work; contributors consent to use in EarthOnline
  without restriction
- **Notes:** shortfalls, retakes pending, files covered by another set

### Synthesised assets template

- **Date:**
- **Author:**
- **Method:** (e.g. "Web Audio offline render, script at
  `scripts/audio/synth-impacts.mjs`" or "layered from originals listed
  above")
- **Files produced:**
- **Licence:** original

### CC0 asset template

- **File(s) in repo:**
- **Source:** (library / archive name, item id or title)
- **Retrieved:** date
- **Licence:** CC0 1.0 (link to the dedication as shown at the source)
- **Modifications:**

## Ledger

### Glass removal effect

- **File shipped:** `public/assets/weapons/audio-v1/impact-glass-removed-1.wav`.
- **Source:** Till Behrend, *Glass Break*, submitted by TinyWorlds with author permission:
  https://opengameart.org/content/glass-break.
- **Retrieved:** 2026-10-07.
- **Licence:** CC0 1.0 Universal, https://creativecommons.org/publicdomain/zero/1.0/.
- **Modifications:** full effect decoded to mono PCM at 48 kHz, peak normalized
  to 0.7, terminal 30 ms fade, metadata stripped. Built by
  `scripts/audio/build-glass-recording-v1.mjs`; shipping and source hashes are
  generated in the weapon bank's `provenance.json`.
- **Scope:** falling-glass effect for newly removed tempered vehicle panes.
  The source does not identify its recording technique or automotive material.
  Retained laminate fracture and subsequent strikes use original project DSP.
  The source WAV stays in off-repository storage.

| File | Origin | Session / source | Added |
|---|---|---|---|
| _(none yet)_ | | | |

<!-- synth-impacts-ledger -->
### Synthesised impact bank

- **Date:** 2026-09-02
- **Author:** EarthOnline audio pipeline (in-house, automated)
- **Method:** Offline render in Node from seeded noise and oscillators, script at
  `scripts/audio/synth-impacts.mjs` (recipes from `impacts/IMPACT_SPEC.md` §2:
  layered noise bursts, resonant sine bodies, band-pass-swept crumples with
  staircase amplitude, partial-cloud glass, resonant pink-noise scrapes,
  filtered square-wave horns). Normalised with `ffmpeg loudnorm`, encoded
  `libopus` 48 kHz mono with `-map_metadata -1`. No recordings, no samples,
  no third-party material of any kind.
- **Files produced:** `impacts/*.ogg` (44 files, 296.1 kB) and `impacts/manifest.json`:
  - `impacts/thud-1-01.ogg` 2407 B, 0.36 s, I=-22.1 LUFS (target -22), TP -5 dBTP
  - `impacts/thud-1-02.ogg` 1683 B, 0.36 s, I=-22.1 LUFS (target -22), TP -5.9 dBTP
  - `impacts/thud-1-03.ogg` 2417 B, 0.36 s, I=-22 LUFS (target -22), TP -5.8 dBTP
  - `impacts/thud-1-04.ogg` 3148 B, 0.36 s, I=-21.8 LUFS (target -22), TP -5.8 dBTP
  - `impacts/thud-2-01.ogg` 4026 B, 0.46 s, I=-18.9 LUFS (target -19), TP -3.3 dBTP
  - `impacts/thud-2-02.ogg` 3523 B, 0.46 s, I=-19 LUFS (target -19), TP -3 dBTP
  - `impacts/thud-2-03.ogg` 1991 B, 0.46 s, I=-19.1 LUFS (target -19), TP -2.6 dBTP
  - `impacts/thud-2-04.ogg` 3486 B, 0.46 s, I=-22.2 LUFS (target -19), TP -3.4 dBTP
  - `impacts/thud-3-01.ogg` 4661 B, 0.66 s, I=-17.4 LUFS (target -17), TP -0.9 dBTP
  - `impacts/thud-3-02.ogg` 4280 B, 0.66 s, I=-17.7 LUFS (target -17), TP -1.4 dBTP
  - `impacts/thud-3-03.ogg` 5217 B, 0.66 s, I=-17.4 LUFS (target -17), TP -0.9 dBTP
  - `impacts/thud-3-04.ogg` 2788 B, 0.66 s, I=-17.9 LUFS (target -17), TP -1.7 dBTP
  - `impacts/car-body-01.ogg` 6737 B, 0.81 s, I=-16.5 LUFS (target -16), TP -0.8 dBTP
  - `impacts/car-body-02.ogg` 7367 B, 0.81 s, I=-16.9 LUFS (target -16), TP -1.8 dBTP
  - `impacts/car-body-03.ogg` 7508 B, 0.81 s, I=-16.3 LUFS (target -16), TP -1 dBTP
  - `impacts/car-body-04.ogg` 7426 B, 0.81 s, I=-16.4 LUFS (target -16), TP -0.9 dBTP
  - `impacts/crunch-1-01.ogg` 5839 B, 0.46 s, I=-19.9 LUFS (target -20), TP -2.5 dBTP
  - `impacts/crunch-1-02.ogg` 5819 B, 0.46 s, I=-20.1 LUFS (target -20), TP -3 dBTP
  - `impacts/crunch-1-03.ogg` 5783 B, 0.46 s, I=-19.9 LUFS (target -20), TP -2.9 dBTP
  - `impacts/crunch-1-04.ogg` 5766 B, 0.46 s, I=-20.2 LUFS (target -20), TP -2.8 dBTP
  - `impacts/crunch-2-01.ogg` 8101 B, 0.91 s, I=-18.7 LUFS (target -17), TP -1.7 dBTP
  - `impacts/crunch-2-02.ogg` 7422 B, 0.91 s, I=-18.9 LUFS (target -17), TP -1.6 dBTP
  - `impacts/crunch-2-03.ogg` 5194 B, 0.91 s, I=-17.9 LUFS (target -17), TP -1.8 dBTP
  - `impacts/crunch-2-04.ogg` 7179 B, 0.91 s, I=-18 LUFS (target -17), TP -1.9 dBTP
  - `impacts/crunch-3-01.ogg` 13276 B, 1.51 s, I=-16.9 LUFS (target -15), TP -1.4 dBTP
  - `impacts/crunch-3-02.ogg` 12771 B, 1.51 s, I=-19.1 LUFS (target -15), TP -1.5 dBTP
  - `impacts/crunch-3-03.ogg` 13770 B, 1.51 s, I=-17.7 LUFS (target -15), TP -1.3 dBTP
  - `impacts/crunch-3-04.ogg` 14248 B, 1.51 s, I=-18.8 LUFS (target -15), TP -1.2 dBTP
  - `impacts/glass-01.ogg` 12471 B, 1.21 s, I=-16 LUFS (target -16), TP -1.8 dBTP
  - `impacts/glass-02.ogg` 12488 B, 1.24 s, I=-16 LUFS (target -16), TP -2.3 dBTP
  - `impacts/glass-03.ogg` 11774 B, 1.11 s, I=-16 LUFS (target -16), TP -0.6 dBTP
  - `impacts/glass-04.ogg` 10967 B, 1.04 s, I=-16 LUFS (target -16), TP -1.1 dBTP
  - `impacts/scrape-loop-01.ogg` 10194 B, 1.63 s, I=-20.8 LUFS (target -20), TP -6 dBTP
  - `impacts/scrape-loop-02.ogg` 9845 B, 1.59 s, I=-20.7 LUFS (target -20), TP -7.2 dBTP
  - `impacts/scrape-loop-03.ogg` 7793 B, 1.23 s, I=-20.5 LUFS (target -20), TP -5.8 dBTP
  - `impacts/scrape-shot-01.ogg` 4990 B, 0.49 s, I=-18 LUFS (target -18), TP -1.9 dBTP
  - `impacts/scrape-shot-02.ogg` 5820 B, 0.62 s, I=-18.4 LUFS (target -18), TP -2.3 dBTP
  - `impacts/scrape-shot-03.ogg` 4528 B, 0.51 s, I=-18 LUFS (target -18), TP -3.7 dBTP
  - `impacts/honk-compact-01.ogg` 3127 B, 0.46 s, I=-16 LUFS (target -16), TP -11.2 dBTP
  - `impacts/honk-compact-02.ogg` 8699 B, 1.17 s, I=-16 LUFS (target -16), TP -12.8 dBTP
  - `impacts/honk-sedan-01.ogg` 4253 B, 0.56 s, I=-16.1 LUFS (target -16), TP -8.9 dBTP
  - `impacts/honk-sedan-02.ogg` 9764 B, 1.25 s, I=-16 LUFS (target -16), TP -10.2 dBTP
  - `impacts/honk-van-01.ogg` 3752 B, 0.53 s, I=-16.1 LUFS (target -16), TP -9.8 dBTP
  - `impacts/honk-van-02.ogg` 8916 B, 1.19 s, I=-16 LUFS (target -16), TP -10.6 dBTP
- **Licence:** original
- **Notes:** placeholder-grade per spec for thud tiers 2–3 and crunch tiers 2–3
  (the spec prefers in-house recordings there); horns and glass are the
  spec's preferred synthesised versions. Regenerate with `npm run audio:synth-impacts`.
<!-- synth-impacts-ledger -->

<!-- recorded-broadcast-ledger -->
### Recorded radio stations (radio-v1)

- **File(s) in repo:** `public/assets/audio/radio-v1/<station>/<track>.ogg` (21 files, listed below) and
  `public/assets/audio/radio-v1/manifest.json` (per-file sha256, source sha256, measured loudness).
- **Source:** FreePD catalogue (https://freepd.com/, site closed; retrieved from the public mirror
  https://a.ocv.me/pub/stuff/music/freepd/, whose README reproduces FreePD's CC0 statement). Track titles below are
  the catalogue's own titles. Station names (Fog City FM, Mission Lounge, Sunset Hits, After Dark, Route One) are
  original to this project.
- **Retrieved:** 2026-10-18 (owner-approved CC0 downloads for the audio pass).
- **Licence:** CC0 1.0 Universal, https://creativecommons.org/publicdomain/zero/1.0/.
- **Modifications:** leading/trailing silence trimmed (-55 dB), two-pass linear loudness normalisation to
  -16 LUFS / -1.5 dBTP, resampled to 48 kHz stereo, 0.25 s fade-in and 1.2 s fade-out, encoded Ogg Opus 96 kb/s
  with `-map_metadata -1` (embedded cover art dropped). Script: `scripts/audio/build-recorded-broadcast.py`.
- **Files:**
  - `public/assets/audio/radio-v1/fog-city-fm/arpent.ogg` - "Arpent" (FreePD `electronic/Arpent.mp3`), 161.9 s, 2312787 B
  - `public/assets/audio/radio-v1/fog-city-fm/beat-one.ogg` - "Beat One" (FreePD `electronic/Beat One.mp3`), 180.0 s, 2001877 B
  - `public/assets/audio/radio-v1/fog-city-fm/goodnightmare.ogg` - "Goodnightmare" (FreePD `electronic/Goodnightmare.mp3`), 239.7 s, 2939790 B
  - `public/assets/audio/radio-v1/fog-city-fm/hear-what-they-say.ogg` - "Hear What They Say" (FreePD `electronic/Hear What They Say.mp3`), 133.8 s, 1963622 B
  - `public/assets/audio/radio-v1/fog-city-fm/meditating-beat.ogg` - "Meditating Beat" (FreePD `electronic/Meditating Beat.mp3`), 157.3 s, 2188294 B
  - `public/assets/audio/radio-v1/mission-lounge/funkeriffic.ogg` - "Funkeriffic" (FreePD `jazz/Funkeriffic.mp3`), 206.8 s, 2606915 B
  - `public/assets/audio/radio-v1/mission-lounge/groovin.ogg` - "Groovin" (FreePD `jazz/Groovin.mp3`), 240.1 s, 3034392 B
  - `public/assets/audio/radio-v1/mission-lounge/martini-sunset.ogg` - "Martini Sunset" (FreePD `jazz/Martini Sunset.mp3`), 171.6 s, 2250591 B
  - `public/assets/audio/radio-v1/mission-lounge/fake-it-til-you-fake-it.ogg` - "Fake It Til You Fake It" (FreePD `jazz/Fake It Til You Fake It.mp3`), 171.8 s, 2143092 B
  - `public/assets/audio/radio-v1/sunset-hits/city-sunshine.ogg` - "City Sunshine" (FreePD `upbeat/City Sunshine.mp3`), 181.8 s, 2729328 B
  - `public/assets/audio/radio-v1/sunset-hits/funshine.ogg` - "Funshine" (FreePD `upbeat/Funshine.mp3`), 161.5 s, 1986059 B
  - `public/assets/audio/radio-v1/sunset-hits/screen-saver.ogg` - "Screen Saver" (FreePD `upbeat/Screen Saver.mp3`), 176.6 s, 2359765 B
  - `public/assets/audio/radio-v1/sunset-hits/pickled-pink.ogg` - "Pickled Pink" (FreePD `upbeat/Pickled Pink.mp3`), 174.3 s, 2530933 B
  - `public/assets/audio/radio-v1/after-dark/sunday-dub.ogg` - "Sunday Dub" (FreePD `misc/Sunday Dub.mp3`), 207.2 s, 2826200 B
  - `public/assets/audio/radio-v1/after-dark/drop-point.ogg` - "Drop Point" (FreePD `misc/Drop Point.mp3`), 182.1 s, 2213496 B
  - `public/assets/audio/radio-v1/after-dark/a-good-bass-for-gambling.ogg` - "A Good Bass for Gambling" (FreePD `misc/A Good Bass for Gambling.mp3`), 154.8 s, 1735662 B
  - `public/assets/audio/radio-v1/after-dark/3-am-west-end.ogg` - "3 am West End" (FreePD `misc/3 am West End.mp3`), 291.1 s, 3963396 B
  - `public/assets/audio/radio-v1/route-one/jethro-on-the-run.ogg` - "Jethro on the Run" (FreePD `country/Jethro on the Run.mp3`), 227.9 s, 2824751 B
  - `public/assets/audio/radio-v1/route-one/trip-up-north.ogg` - "Trip Up North" (FreePD `country/Trip Up North.mp3`), 166.8 s, 2121993 B
  - `public/assets/audio/radio-v1/route-one/hillbilly-swing.ogg` - "Hillbilly Swing" (FreePD `world/Hillbilly Swing.mp3`), 164.9 s, 2382289 B
  - `public/assets/audio/radio-v1/route-one/still-pickin.ogg` - "Still Pickin" (FreePD `upbeat/Still Pickin.mp3`), 295.1 s, 3880796 B

### Recorded street, rain, rotor and crowd-walla loops (city-v2)

- **File(s) in repo:** `public/assets/audio/city-v2/*.ogg` (8 files, listed below) and
  `public/assets/audio/city-v2/manifest.json` (per-file sha256, per-source sha256, windows, weights, measured level).
- **Source:** BigSoundBank (https://bigsoundbank.com/, recordings by Joseph Sardin); each item page checked for
  "CC0 (public domain)" (the off-repo download index records the page per id).
- **Retrieved:** 2026-10-18.
- **Licence:** CC0 1.0 Universal, https://creativecommons.org/publicdomain/zero/1.0/.
- **Modifications:** excerpted windows (chosen from a 3 s short-term loudness profile to skip one-off spikes),
  high-pass, optional low-pass, per-source RMS equalisation then weighted mix, equal-power loop crossfade, RMS
  normalised to 0.14 (the level the in-game renderers target; the two walla loops instead to the RMS of the
  city-v1 crowd clip each replaces, 0.0403 and 0.0653), look-ahead limiter at 0.89, 48 kHz, encoded Ogg Opus
  (96 kb/s stereo, 64 kb/s mono) with `-map_metadata -1`. Script: `scripts/audio/build-recorded-broadcast.py`.
- **Files:**
  - `public/assets/audio/city-v2/street-day.ogg` - daytime street bed: crossroads traffic under a pedestrian street; 60 s, 2 ch, 643166 B; BigSoundBank 0608 https://bigsoundbank.com/street-and-roads-s0608.html from 60 s x0.75; BigSoundBank 0641 https://bigsoundbank.com/london-street-s0641.html from 60 s x0.6
  - `public/assets/audio/city-v2/street-night.ogg` - night street bed: 1 am square from a balcony over a distant freeway rumble; 60 s, 2 ch, 612239 B; BigSoundBank 0680 https://bigsoundbank.com/paris-by-night-s0680.html from 15 s x0.8; BigSoundBank 0122 https://bigsoundbank.com/highway-s0122.html from 6 s x0.35 lowpass 700 Hz
  - `public/assets/audio/city-v2/rain-concrete.ogg` - rain body: rain on concrete; 30 s, 1 ch, 372414 B; BigSoundBank 1289 https://bigsoundbank.com/rain-on-concrete-s1289.html from 3 s x1.0
  - `public/assets/audio/city-v2/rain-car-roof.ogg` - roof drumming: rain on a car roof; 30 s, 1 ch, 265110 B; BigSoundBank 1293 https://bigsoundbank.com/rain-on-car-roof-s1293.html from 10 s x1.0
  - `public/assets/audio/city-v2/rain-puddle.ogg` - ground layer: rain on a puddle; 30 s, 1 ch, 305948 B; BigSoundBank 1290 https://bigsoundbank.com/rain-on-puddle-s1290.html from 5 s x1.0
  - `public/assets/audio/city-v2/helicopter-rotor.ogg` - police helicopter rotor loop; 7.5 s, 1 ch, 55192 B; BigSoundBank 0113 https://bigsoundbank.com/helicopter-transition-s0113.html from 7.0 s x1.0
  - `public/assets/audio/city-v2/walla-plaza.ogg` - crowd walla: people crossing a pedestrian square; 30 s, 1 ch, 214436 B; BigSoundBank 0526 https://bigsoundbank.com/pedestrian-place-s0526.html from 21 s x1.0 lowpass 5200 Hz; BigSoundBank 0527 https://bigsoundbank.com/small-pedestrian-street-s0527.html from 12 s x0.45 lowpass 5200 Hz
  - `public/assets/audio/city-v2/walla-street.ogg` - crowd walla: a narrow shopping street; 30 s, 1 ch, 205359 B; BigSoundBank 0527 https://bigsoundbank.com/small-pedestrian-street-s0527.html from 33 s x1.0 lowpass 5200 Hz; BigSoundBank 0526 https://bigsoundbank.com/pedestrian-place-s0526.html from 52 s x0.5 lowpass 5200 Hz

### Recorded vehicle and impact one-shots, tyre squeal loop (sfx-v1)

- **File(s) in repo:** `public/assets/audio/sfx-v1/*.ogg` (7 files, listed below) and
  `public/assets/audio/sfx-v1/manifest.json` (per-file sha256, source sha256, window, level reference, measured level).
- **Source:** BigSoundBank (https://bigsoundbank.com/, recordings by Joseph Sardin); each item page checked for
  "CC0 (public domain)" (the off-repo download index records the page per id).
- **Retrieved:** 2026-10-18.
- **Licence:** CC0 1.0 Universal, https://creativecommons.org/publicdomain/zero/1.0/.
- **Modifications:** excerpted window, high-pass, cosine fade-out; one-shots level-matched on active RMS (mean power
  of the four loudest 100 ms windows) to the synthesised clip each stands in for (named per file in the manifest);
  the squeal loop has an equal-power loop crossfade and RMS 0.099 (the synthesised squeal band it is blended with);
  look-ahead limiter at 0.89, 48 kHz mono, Ogg Opus 64 kb/s with `-map_metadata -1`.
  Script: `scripts/audio/build-recorded-broadcast.py --only sfx`.
- **Files:**
  - `public/assets/audio/sfx-v1/horn-heavy-press.ogg` - truck/bus air horn, one press; 1.256 s, 1 ch, 13875 B; BigSoundBank 3594 https://bigsoundbank.com/bus-horn-s3594.html from 0.0 s, level matched to `artifacts/audio/impacts/honk-van-01.ogg`
  - `public/assets/audio/sfx-v1/horn-heavy-toot.ogg` - truck air horn, one short blast; 1.107 s, 1 ch, 10373 B; BigSoundBank 2721 https://bigsoundbank.com/truck-horn-s2721.html from 0.0 s, level matched to `artifacts/audio/impacts/honk-van-01.ogg`
  - `public/assets/audio/sfx-v1/horn-heavy-lean.ogg` - bus horn, three blasts (leaned on); 1.756 s, 1 ch, 18727 B; BigSoundBank 3593 https://bigsoundbank.com/bus-horns-s3593.html from 0.0 s, level matched to `artifacts/audio/impacts/honk-van-02.ogg`
  - `public/assets/audio/sfx-v1/air-brake.ogg` - heavy vehicle air-brake release as it stops; 0.806 s, 1 ch, 6577 B; BigSoundBank 1490 https://bigsoundbank.com/pneumatic-brake-released-s1490.html from 0.05 s, level matched to `public/audio/city-v1/parking-brake.ogg`
  - `public/assets/audio/sfx-v1/glass-burst.ogg` - window glass bursting (crash, tier 3); 1.817 s, 1 ch, 15252 B; BigSoundBank 0148 https://bigsoundbank.com/glass-burst-s0148.html from 0.0 s, level matched to `artifacts/audio/impacts/glass-01.ogg`
  - `public/assets/audio/sfx-v1/glass-mirror.ogg` - mirror glass exploding (crash, tier 3); 1.427 s, 1 ch, 12053 B; BigSoundBank 0387 https://bigsoundbank.com/mirror-explodes-s0387.html from 0.0 s, level matched to `artifacts/audio/impacts/glass-01.ogg`
  - `public/assets/audio/sfx-v1/tyre-squeal.ogg` - player car tyre squeal loop (lock-up, drift, wheelspin); 6.0 s, 1 ch, 37398 B; BigSoundBank 0500 https://bigsoundbank.com/tire-squeal-s0500.html from 4.0 s
<!-- recorded-broadcast-ledger -->
