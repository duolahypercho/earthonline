# Pedestrian bark bank — specification

Owner: audio. Consumed by the Phase 3 sample layer in `src/audio.js` (or
`src/audio/barks.js`) described in `Docs/SINGLE_PLAYER_FEEL_PLAN.md`.
Provenance rules: `artifacts/audio/PROVENANCE.md`. Recording plan:
`RECORDING_SCRIPT.md`. Loader shape: `manifest.template.json`.

Tone rule (from the plan): human, not cartoonish. A `bump` may be funny; a
`scream` must not be. The heavier tiers only work if the lighter ones are
underplayed, so every direction note below errs toward restraint.

## 1. Categories and impact-tier mapping

Bark category is chosen by the impact bus tier (`bump` / `stagger` /
`knockdown` / `thrown`) plus the agent's behaviour state. The runtime picks
one category per event, never two.

| Category | Triggered by | Who says it | Length target | Variants |
|---|---|---|---|---|
| `bump` | `bump` tier (walk into someone) | the agent that was bumped | 0.3–0.8 s | 8 |
| `hurt-light` | `stagger` tier (sprint collision, weak punch) | the agent hit | 0.4–0.9 s | 8 |
| `hurt-heavy` | `knockdown` tier (strong punch, car 10–20 km/h) | the agent hit | 0.6–1.4 s | 7 |
| `scream` | `thrown` tier (car 30 km/h+), or `knockdown` from a vehicle | the agent hit | 0.9–1.8 s | 6 |
| `angry` | `confront` behaviour after any tier | the agent walking toward the player | 0.6–1.5 s | 8 |
| `flee` | `flee` behaviour after `knockdown`+ nearby | bystanders running | 0.5–1.2 s | 7 |
| `gasp` | `flinch` bystander reaction within 15 m of `knockdown`+ | bystanders | 0.3–0.9 s | 8 |
| `honk` | car-on-car contact, near-miss with player | the *other* vehicle (driver-set, not voice-set) | 0.4–1.5 s | 6 |
| `shout` | driver after car-on-body or being rammed | the *other* vehicle's driver | 0.6–1.4 s | 6 |

`honk` is not a voice; it is listed here because it shares the bark
scheduler (cooldown, limiter, panner). Its samples live in the impact bank
(`../impacts/IMPACT_SPEC.md`) and are referenced from the same manifest.

### 1.1 Line variants

Wording is a guide for the performer, not a subtitle. Lines are short so
that the recording can be trimmed hard; nothing in this bank exceeds 1.8 s.
Vocalisations in `[brackets]` are non-verbal.

**bump** (8) — mild, surprised, slightly annoyed; volume of normal speech
1. "Hey."
2. "Whoa — watch it."
3. "[short grunt] Sorry — no, *you* sorry."
4. "Excuse me?"
5. "Oi."
6. "Careful."
7. "[breathy 'tsk'] Come on."
8. "Mind yourself."

**hurt-light** (8) — sharp intake, genuine but small pain, no words needed
1. "[sharp 'ah!']"
2. "Ow — hey!"
3. "[grunt] What the—"
4. "Agh. Seriously?"
5. "[wince] Hff."
6. "Ow. *Ow.*"
7. "[hiss through teeth]"
8. "Hey! Back off."

**hurt-heavy** (7) — real, winded, lower in pitch; the wind is knocked out
1. "[winded 'uhh' on impact]"
2. "[cry cut off by the ground] Ah—"
3. "[groan] ...god..."
4. "[breath punched out] Hhk—"
5. "[heavy grunt, then a low moan]"
6. "Ahh — my— [trails off]"
7. "[a sob-like exhale, no words]"

**scream** (6) — full voice, but *frightened* not theatrical; think of
seeing a car too late. One breath.
1. "[scream, rising, cut off]"
2. "NO— [scream]"
3. "[scream falling into a cry]"
4. "[short shriek, then silence]"
5. "[scream with a word buried: 'wait—']"
6. "[gasp then scream, cut]"

**angry** (8) — confronting the player; speech, not shouting; anger held in
1. "What is wrong with you?"
2. "Hey. HEY. Come here."
3. "You think that's funny?"
4. "Do that again. Go on."
5. "Are you out of your mind?"
6. "I'm talking to you."
7. "Who do you think you are?"
8. "Get back here!"

**flee** (7) — bystander running; breath in the line, some directed at
others
1. "Go, go, go—"
2. "[panicked] Get out of the way!"
3. "Run!"
4. "Someone call— [fades as they run]"
5. "Oh no, no no no."
6. "Move! MOVE!"
7. "[short yelp then running breath]"

**gasp** (8) — bystander flinch; mostly non-verbal
1. "[sharp gasp]"
2. "Oh my god."
3. "[inhale] Jesus."
4. "Oh—!"
5. "[hand-to-mouth gasp, muffled]"
6. "Did you see—"
7. "[gasp] Is he okay?"
8. "[small 'oh' with a wince]"

**shout** (6) — driver, through a window; angry, a little distant
1. "Hey! Are you blind?"
2. "What are you doing?!"
3. "Get off the road!"
4. "You idiot!"
5. "Look where you're going!"
6. "[horn-lean] Move it!"

Each set records every line once per category; the manifest may ship fewer
variants for a set if takes are unusable (minimum 4 per category per set).

## 2. Voice sets and archetype assignment

Three sets. They are pitch/register groupings, not character voices; the
per-identity pitch offset gives individuals inside a set their own voice.

| Set | Register | Performer note | Assigned identities |
|---|---|---|---|
| `a` | low (approx. 85–150 Hz speaking fundamental) | the lowest natural voice in the room, no forced growl | `presentation: masculine`; also any `neutral` identity with body `strong` or `heavy`; authored bodies `masculine-average-v1`, `worker-masculine-v1` |
| `b` | mid (approx. 120–200 Hz) | the "middle" voice; can be either performer with the widest range | `presentation: neutral` (except the `strong`/`heavy` cases above); fallback set when a category is missing in `a` or `c` |
| `c` | high (approx. 165–260 Hz) | the highest natural voice; still an adult, no character voice | `presentation: feminine`; also any `neutral` identity with body `slim` or `short`; authored body `commuter-feminine-v1` |

Runtime resolution (proposal for the implementer; identity fields from
`src/simulation/pedestrians/npc-generator.js` `generateCharacterRecipe`):

```
set  = presentation === 'masculine' ? 'a'
     : presentation === 'feminine'  ? 'c'
     : (body in {strong, heavy})     ? 'a'
     : (body in {slim, short})       ? 'c'
     : 'b'
pitchOffset = lerp(-0.08, +0.08, npcIdentityRandom(seed, 'voice:pitch'))
              + (shape.age > 0 ? -0.02 * shape.age : 0)        // older reads lower
              + (shape.height - 0) * -0.015                       // taller reads slightly lower
pitchOffset = clamp(pitchOffset, -0.10, +0.10)
```

`pitchOffset` is applied as `playbackRate = 2 ** (pitchOffset)` scaled so
that ±8 % is ±0.08 in the manifest (i.e. `playbackRate = 1 + pitchOffset`).
Playback-rate shifting also shortens/lengthens the sample; at ±10 % this is
inaudible for barks and costs nothing, which is why no formant shifter is
specified.

Drivers use `shout` from the set matching the driver's identity if a driver
identity exists; otherwise round-robin `a`/`b`/`c` seeded by the vehicle id.

## 3. Delivery format

| Property | Value |
|---|---|
| Container / codec | Ogg, Opus (`.ogg`). See decision note below. |
| Sample rate | 48 000 Hz |
| Channels | mono |
| Bitrate | 40 kb/s VBR voice mode (`-application voip` is *not* used; use `-application audio`) |
| Integrated loudness (per file) | see table below, measured with ITU-R BS.1770 (`ffmpeg loudnorm`) |
| True peak | ≤ −1.0 dBTP |
| Head / tail | 5 ms fade-in, 30 ms fade-out; no leading silence > 10 ms; tail silence ≤ 40 ms |
| Noise floor | ≤ −60 dBFS RMS in the trimmed tail |

Per-category loudness targets (integrated LUFS, per file, before in-game
gain). Deliberately quieter for chatter and louder for pain so that the
runtime's category gain can stay near unity:

| Category | Target LUFS | Tolerance |
|---|---|---|
| bump, gasp | −23 | ±1.5 |
| hurt-light, flee, angry, shout | −20 | ±1.5 |
| hurt-heavy | −18 | ±1.5 |
| scream | −16 | ±1.5 |
| honk | −16 | ±1.0 |

**Format decision note.** The plan says "ogg". Two codecs fit that
container: Vorbis and Opus. Opus is chosen because (a) it is natively
48 kHz, matching the sample-rate requirement exactly, (b) it is ~30 % smaller
than Vorbis at equal speech quality, and (c) the local ffmpeg on the build
machine has `libopus` but no `libvorbis`. All current desktop and mobile
browsers decode Ogg/Opus through `decodeAudioData`. If a target browser is
later found that cannot, re-encode with `-c:a libvorbis -q:a 3` from the
retained WAV masters; nothing else changes.

### 3.1 Naming and layout

```
artifacts/audio/barks/
  <set>-<category>-<nn>.ogg      e.g. a-hurt-heavy-03.ogg, c-gasp-07.ogg
  manifest.json                  (built from manifest.template.json)
  masters/                       (WAV 48k/24-bit, gitignored, kept locally)
```

- `<set>` ∈ `a|b|c`, `<category>` exactly as in section 1 (hyphenated),
  `<nn>` two-digit, 1-based, contiguous.
- Honks are `<driver>-honk-<nn>.ogg` under `artifacts/audio/impacts/` with
  `<driver>` ∈ `compact|sedan|van` (vehicle class, see impact spec).

### 3.2 Budget

64 bark variants × 3 sets = 192 voice files. At 40 kb/s Opus and a mean
length of 0.8 s that is ~4 kB each → ~770 kB. With the impact bank
(~900 kB, see impact spec) the whole Phase 3 payload sits at ~1.7 MB,
inside the 2 MB cap. If a set overruns, drop variants 7–8 from `bump`,
`gasp` and `angry` first.

## 4. Runtime behaviour numbers

These are the tuning values the implementer should start from; they are the
values the DoD "limiter holds under a 20-pedestrian pile-up" check assumes.

### 4.1 Per-agent cooldown

| Rule | Value |
|---|---|
| Same agent, any category | 1.2 s minimum between barks |
| Same agent, same category | 4.0 s |
| Same agent, `scream` or `hurt-heavy` | one per reaction; no repeat until the reaction record clears |
| Same agent, `angry` | 2.5 s while in `confront`; max 3 per confrontation |
| Variant selection | shuffle-bag per (agent set, category); never the same variant twice in a row within one agent, and not the same variant as any bark played in the last 2 s globally |

### 4.2 Global limiter (crowd cascade guard)

| Rule | Value |
|---|---|
| Hard cap | 3 barks started per rolling 0.5 s window |
| Priority when capped | `scream` > `hurt-heavy` > `hurt-light` > `angry` > `flee` > `gasp` > `bump`; higher priority may pre-empt (fade 30 ms) the most recent lower-priority bark if the cap is hit |
| Distance cull | events farther than 28 m from the listener never enter the queue |
| Concurrent voices | ≤ 8 bark voices + ≤ 6 impact voices |
| Gasp/flee crowd fan-out | after a `knockdown`+ event, at most 4 bystander barks in the following 2 s regardless of how many flinch; choose the 4 closest |
| Ducking | when a `scream` plays, the ambience/engine bus ducks −4 dB for 0.6 s (attack 20 ms, release 400 ms) |

Bus layout (proposal): `bark bus` → `DynamicsCompressor` (threshold −14,
knee 10, ratio 6, attack 3 ms, release 180 ms) → master. Reuse the compressor
shape already in `createCombatAudio` in `src/audio.js` rather than adding
a new one.

### 4.3 Spatialisation (`PannerNode`)

| Property | Value |
|---|---|
| `panningModel` | `HRTF` for ≤ 8 simultaneous bark voices; fall back to `equalpower` if the frame cost is measurable on the low-end profile |
| `distanceModel` | `inverse` |
| `refDistance` | 2.0 m |
| `maxDistance` | 30 m (cull at 28 m, above, so nothing plays at the clamp) |
| `rolloffFactor` | 1.3 |
| `coneInnerAngle` / `coneOuterAngle` / `coneOuterGain` | 360 / 360 / 1 (voices are omnidirectional at this scale) |
| Listener | camera position, camera forward/up; update once per frame from the render camera, not from the player capsule |
| Doppler | none |

Honks use `refDistance` 4.0, `rolloffFactor` 1.0, `maxDistance` 60, cull at
55 m — a horn carries.

### 4.4 Category gain (runtime multiplier on the manifest `gain`)

Start at 1.0 for everything except `bump` 0.85 and `gasp` 0.9. These are the
knobs to turn during the five-minute test, not the file loudness.

## 5. Acceptance

- Every category has ≥ 4 usable variants per set; ≥ 6 preferred.
- Every file passes the format table (a checker script can assert with
  `ffprobe` + `ffmpeg -af ebur128`).
- Listening check: play each `scream` and `hurt-heavy` file once in
  isolation at −20 dB; if a variant makes the listener smile it goes back
  for a retake. The light tiers are allowed to be funny; these are not.
- No file references, quotes, or imitates any third-party media.
