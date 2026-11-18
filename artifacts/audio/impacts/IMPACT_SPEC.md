# Impact bank — specification

Companion to `../barks/BARK_SPEC.md`. Same format rules (Ogg/Opus, 48 kHz,
mono, true peak ≤ −1 dBTP), same scheduler and `PannerNode` values unless
overridden below. Every file is original or CC0 and logged in
`../PROVENANCE.md`.

Impact sounds are triggered by the impact bus tier (`bump` / `stagger` /
`knockdown` / `thrown`) and the pair of `kinds` on the event. A bark and an
impact may play for the same event; they are separate voices.

## 1. Budget

The Phase 3 cap is 2 MB total for barks + impacts. Barks are planned at
~770 kB, leaving **≤ 1.2 MB** for this bank; the target below is ~900 kB
so there is headroom for one extra category.

| Category | Files | Length | Bitrate | Approx. size |
|---|---|---|---|---|
| body thud, 3 tiers × 4 variants | 12 | 0.25–0.6 s | 48 kb/s | 30 kB |
| car-on-body, 4 variants | 4 | 0.5–1.0 s | 56 kb/s | 20 kB |
| car-on-car crunch, 3 tiers × 4 | 12 | 0.4–1.6 s | 64 kb/s | 90 kB |
| glass, 4 variants | 4 | 0.6–1.4 s | 64 kb/s | 35 kB |
| scrape, 3 loops + 3 one-shots | 6 | 1.0–2.0 s loops, 0.3–0.8 s shots | 56 kb/s | 60 kB |
| horn / honk, 3 vehicle classes × 2 | 6 | 1.0–2.7 s | 48 kb/s | 95 kB |
| **total** | **44** | | | **~330 kB** |

The horn row is the one that grew: a tap-length honk reads as a lock chirp
rather than a horn, so both variants are now a held blast (a short press and
a leaned-on one). That is the whole of the difference between the ~275 kB
first plan and the ~330 kB above.

That is still far under budget on purpose: impact sounds are broadband and
survive compression less gracefully than voice, so if listening tests show
Opus smearing the crunch transients, raise the impact bitrates to 96 kb/s
(~500 kB) without touching the plan.

Naming: `<category>-<tier|class>-<nn>.ogg`, e.g. `thud-2-03.ogg`,
`crunch-3-01.ogg`, `honk-sedan-02.ogg`, `scrape-loop-01.ogg`.

## 2. Categories

Loudness targets are integrated LUFS per file. Tiers are numbered 1–3 =
light/medium/heavy and map to impact tiers as noted.

### 2.1 Body thud (`thud-<1|2|3>-<nn>`)

Pedestrian body hitting ground or a wall; also the player landing after a
`thrown` tier.

| Tier | Maps to | Character | LUFS |
|---|---|---|---|
| 1 | `stagger` | soft, mostly cloth and a small chest thump | −22 |
| 2 | `knockdown` | one solid low thump with a short cloth flap | −19 |
| 3 | `thrown` landing | low thump plus a second smaller bounce 80–150 ms later | −17 |

**Record in-house.** A duffel bag or a rolled sleeping bag dropped onto a
carpeted floor from 30 cm / 60 cm / 120 cm is the standard trick; a bag
half full of rice or flour gives the "mass" tier 3 needs. Mic 40 cm from
the impact point, low, pointed at the floor; record 10 drops per tier and
pick the four cleanest. Add a folded leather jacket on top for the cloth
layer. Do not record a person actually falling.

**Synthesise if no room.** Layer (a) a 60–90 Hz sine with a 15 ms attack
and 120 ms exponential decay, (b) a low-passed (400 Hz) noise burst of
40 ms, (c) for tier 3 a repeat of (a) at −8 dB delayed 110 ms. This reads
as "thump" but lacks the cloth; acceptable as a placeholder only.

### 2.2 Car-on-body (`car-body-<nn>`)

Vehicle hitting a pedestrian at `knockdown`+ tiers. Played once per
vehicle-pedestrian pair per reaction. The pedestrian's `scream` or
`hurt-heavy` bark plays in parallel; keep this sound free of any vocal
element so the two never fight.

Character: a dull, heavy thump with a short metallic "bonnet" resonance
(a sheet-metal ring around 180–320 Hz, 200 ms) and no crunch. LUFS −16.

**Record in-house.** Strike a car bonnet (with permission, on a stationary
car, with a padded fist or a rubber mallet through a towel) — the panel
ring is the identifiable part. Layer under it a tier-2 body thud. If no car
is available, a metal baking tray on a folded towel struck with a
towel-wrapped mallet gives a similar ring; pitch it down 3–5 semitones.

**Do not** layer glass into this category; glass is its own event.

### 2.3 Car-on-car crunch (`crunch-<1|2|3>-<nn>`)

| Tier | Relative speed (indicative) | Character | LUFS |
|---|---|---|---|
| 1 | < 10 km/h | bumper tap: one short plastic knock, light panel ring | −20 |
| 2 | 10–30 km/h | panel crumple: knock + 300–500 ms of crackling sheet metal | −17 |
| 3 | > 30 km/h | full crunch: knock, long crumple (0.8–1.2 s), a glass component is **not** included (see 2.4), ends with a small debris tail | −15 |

**Record in-house.** The crumple layer is the hard part and the one that
sells the tier. Options that work with household material:
- an empty aluminium drinks can crushed slowly by hand, pitched down 8–12
  semitones and slowed 1.5×, gives a convincing panel crumple;
- a biscuit tin or a large empty paint tin dented with a mallet gives the
  knock and ring;
- a handful of dry pasta or gravel dropped on a tray gives the debris tail.
Record each element separately and layer in the editor; never try to record
the whole event as one take.

**Synthesise.** Knock: 120 Hz sine, 10 ms attack, 80 ms decay, plus a
band-passed noise click. Crumple: white noise through a band-pass sweep
1.5 kHz → 400 Hz over the duration, amplitude modulated by a random
staircase (12–30 steps/s) to fake the crackle. This is passable for tier 1,
weak for tiers 2–3; record if at all possible.

### 2.4 Glass (`glass-<nn>`)

Window break on car-on-car tier 3, car-on-world at speed, and gunfire on
vehicles (Phase 5+). Character: initial crack, a bright shatter of
0.3–0.5 s, a falling-shards tail. LUFS −16.

**Record in-house.** Do not break real glass. Substitutes that read as
glass on a mic: a handful of small ceramic tiles or old crockery shards
poured from 30 cm onto a ceramic plate over a folded towel; a bag of
tempered-glass phone-screen-protector offcuts if you have them. High-pass at
300 Hz, add a 3 ms transient click at the front from a snapped pencil.

**Synthesise.** Many (60–120) short decaying sine partials between 2 and
9 kHz with random onset over 300 ms, each 30–80 ms long, plus a white
noise burst at onset. This actually works well for glass and is an
acceptable shipping version.

### 2.5 Scrape (`scrape-loop-<nn>`, `scrape-shot-<nn>`)

Vehicle sliding along a wall or another vehicle; also the player sliding
after being thrown. Loops play while contact persists (gain follows
relative speed, cut when speed < 1 m/s); one-shots play at the start of a
scrape. LUFS −20 (loops), −18 (shots).

**Record in-house.** A metal ruler or a saucepan lid dragged along a
concrete step or brick wall, slow and steady, 3 s per take; trim to a
1.0–2.0 s loop with a 30 ms crossfade at the seam (`afade` in/out then
overlap in the editor, or use the `acrossfade` filter). One-shots: the
first 0.4 s of a drag that starts hard.

**Synthesise.** Pink noise through a band-pass at 900–1400 Hz with slow
random centre-frequency drift and a resonant peak (Q ~8) — passable.

### 2.6 Horn / honk (`honk-<compact|sedan|van>-<nn>`)

Played by the *other* vehicle on car-on-car contact and near-misses;
scheduled through the bark scheduler with the `honk` panner overrides
(`refDistance` 4, `maxDistance` 60). Two variants per class: one short
"tap" (0.4–0.6 s), one long "lean" (1.0–1.5 s). LUFS −16.

| Class | Character |
|---|---|
| compact | single tone, higher, ~500–620 Hz |
| sedan | two-tone chord, ~400 + 480 Hz |
| van | lower, slightly rougher, ~330 + 400 Hz |

**Synthesise — preferred.** Real car horns are a nuisance to record and a
synthesised one sounds right. Per tone: square wave through a low-pass at
2.5 kHz, 2 % vibrato at 6 Hz, 15 ms attack, 40 ms release; sum the two
tones at equal gain, add a touch of soft clipping (`tanh`) for the "van"
class. Render offline from a small Node/Web Audio script committed under
`scripts/audio/` so the horns are reproducible.

**Record in-house.** Only if a car is at hand; from 5 m, outdoors, early in
the day. Note in provenance that the recording is original.

## 3. Runtime notes for the implementer

- Impact voices share the bark `DynamicsCompressor` bus; concurrent cap 6.
- Same-pair cooldown: one impact sound per (a, b) pair per 0.4 s; the
  physics solver can emit several contacts per frame for one crash.
- Random variant, never the same twice in a row per category; ±4 %
  playback-rate jitter on every impact one-shot (not on honks).
- `car-body` and `crunch-3` duck ambience/engine −3 dB for 0.5 s.
- Scrape loop gain = clamp((relSpeed − 1) / 8, 0, 1) × category gain,
  updated per frame with `setTargetAtTime` (time constant 50 ms).
- Load lazily after world boot with the barks; the loader must not block
  the first frame (see the boot-time note in the plan).

## 4. Post-processing

Identical to `../barks/RECORDING_SCRIPT.md` §6 with two changes:

- Encode at the per-category bitrate from §1 (`-b:a 48k/56k/64k`).
- Add `-map_metadata -1` on every encode so no session metadata or tags
  reach the repo.

```bash
ffmpeg -i crunch-3-01.wav -af "loudnorm=I=-15:TP=-1.0:LRA=9:linear=true" \
  -ar 48000 -ac 1 -c:a pcm_s24le crunch-3-01.norm.wav
ffmpeg -i crunch-3-01.norm.wav -map_metadata -1 -c:a libopus -b:a 64k -vbr on \
  -application audio -ar 48000 -ac 1 crunch-3-01.ogg
```
