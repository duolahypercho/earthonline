# Bark bank — one-session recording script

Three performers (you + two friends), ~90 minutes, one room. Produces the
192 voice files listed in `BARK_SPEC.md`. Read that spec's section 1 for
the line list; this document is the order of work, the direction notes,
and the post-processing steps.

Every recording made in this session is original work by the performers
and is logged in `artifacts/audio/PROVENANCE.md` before it is used.

## 0. Cast

Assign by natural speaking pitch, not by who wants which part:

| Set | Performer | Register |
|---|---|---|
| `a` | lowest natural voice in the room | low |
| `b` | middle voice (or the person with the widest range) | mid |
| `c` | highest natural voice | high |

Each performer records **all nine voice categories** for their own set. No
one puts on a voice. If two of you sound similar, the runtime's ±8 % pitch
offset still separates individuals; do not try to widen the gap by acting.

## 1. Kit and room (10 min setup, do this before anyone arrives)

- Mic: any USB or XLR condenser / dynamic that gets 48 kHz. A phone in
  voice-memo mode at highest quality is acceptable as a last resort
  (export as WAV/M4A, resample later).
- Position: 15–20 cm from the mouth, slightly off-axis (mic pointed at the
  cheek, not the lips) to survive the plosives in "bump" and the air blasts
  in the screams. Pop filter if you have one; a sock over the mic if not.
- Room: the smallest room with the softest surfaces. A wardrobe with
  clothes in it beats a living room. Hang a duvet behind the performer if
  the room rings when you clap.
- Levels: set input gain so a **normal-volume** "Hey." peaks around
  −18 dBFS. The screams will then peak around −3 to −6 dBFS. Do **not** set
  gain for the scream; if you do, everything else is too quiet and noisy.
  Do a scream test at the set level first and back off if it clips.
- Record 48 kHz / 24-bit WAV, mono. One long file per performer per block
  (you cut it up later); slate each line by saying the category and line
  number ("hurt light, four") before it. Leave 1 s of silence after each
  take.
- Record 10 s of room silence at the start of each performer's block. You
  need it for noise reduction.
- Phones on silent, fridge off if it's next door, everyone else out of the
  room or perfectly still.

## 2. Schedule (~90 min)

Rotate performers per block so nobody strains their voice by doing all the
screams back to back. Warm up 2 min per performer: hum, sirens, ten
sentences read normally.

| Time | Block | Who |
|---|---|---|
| 0:00 | setup check, levels, room tone × 3 | all |
| 0:10 | Block 1 — quiet categories: `bump`, `gasp` | a, then b, then c (≈5 min each) |
| 0:25 | Block 2 — speech categories: `angry`, `shout`, `flee` | a, b, c (≈6 min each) |
| 0:45 | break, water, no throat-clearing (it irritates) | |
| 0:50 | Block 3 — `hurt-light` | a, b, c (≈4 min each) |
| 1:02 | Block 4 — `hurt-heavy` | a, b, c (≈5 min each) |
| 1:17 | Block 5 — `scream` | a, b, c (≈3 min each; hard stop at 6 takes per line) |
| 1:26 | pickups: relisten to anything flagged, re-record | |
| 1:30 | done |

## 3. Take counts

| Category | Takes per line | Why |
|---|---|---|
| bump | 3 | easy, you'll pick the flattest one |
| gasp | 3 | non-verbal, cheap |
| angry | 3 | speech; the third take is usually best |
| shout | 3 | |
| flee | 4 | the running breath is hard to fake sitting down — stand, jog on the spot |
| hurt-light | 4 | the sharp intake tends to be too big; extra takes let you pick the smallest one that still reads |
| hurt-heavy | 5 | this is the hardest register in the session |
| scream | 4–6 | but stop early if the voice tires; a strained scream is a wasted file |

Total ≈ 64 lines × ~3.7 takes × 3 performers ≈ 700 takes; at ~5 s each
including slate and gap that is ~60 min of talking, which fits.

## 4. Direction notes (read aloud to the cast before each block)

**General.** Everything is at the scale of a real street. Imagine the mic
is a stranger two metres away. Nobody projects to a back row. If you find
yourself "doing a character", stop, breathe, and say it like it happened to
you this morning.

**bump.** You just got shouldered. You're mildly annoyed and a little
surprised. Half of these should be almost thrown away. Do one take
where you're actually not that bothered.

**gasp.** You watched someone get hit across the street. The gasp is the
breath in, not a sound out. Keep the mouth close-ish to the mic for the
muffled one (line 5) — cover your mouth with your hand for real.

**angry.** You are walking toward the person who did it. You are not
screaming; you are containing it. Lower and slower reads as more dangerous.
Line 2 ("Hey. HEY.") is the only one with a raised voice.

**shout.** You're in a car with the window down; the other person is
outside. Turn your head 30° off the mic for a natural "through the window"
feel, don't cup your hands.

**flee.** Stand up. Jog on the spot for ten seconds before the take so the
breath is real. The words are secondary to the breath.

**hurt-light.** Something hit you harder than a bump. It's a real "ow" but
you're fine. The most common mistake is too much sound; think of stubbing a
toe in company and trying not to make a scene. For "[hiss through teeth]"
actually clench the teeth.

**hurt-heavy.** This is the tier that has to read as real. You have been
knocked down and the wind has gone out of you. Tips that work:
- Exhale fully, then make the sound on what's left. The "empty lung" sound
  is what a real impact sounds like.
- Get down on the floor for these. Lying on your side changes the
  diaphragm and it will sound right without acting.
- Do not add a moan on top; the moan is its own line (5).
- If a take sounds like a cartoon, it is because it was too loud or too
  long. Shorter and quieter.

**scream.** You saw the car late. Fear, not pain, not drama. Notes:
- One breath. Rising pitch. Cut it off — either by "the ground" (stop
  abruptly) or by running out of air.
- Do not make it long; the file is capped at 1.8 s and the best ones are
  under 1.2 s.
- Six takes maximum per line, then stop; a tired scream sounds fake and
  you will need the voice for pickups.
- Back off the mic 30–40 cm for these takes, keep the input gain where it
  was.

**Everyone.** Between lines, silence. Don't laugh into the tail of a take —
it makes the trim harder and it bleeds into the edit.

## 5. In-session checklist

- [ ] Room tone recorded for each performer (10 s)
- [ ] Levels verified on a scream before Block 1
- [ ] Every take slated ("category, line number")
- [ ] One "flagged for retake" note per performer kept on paper
- [ ] Voice checked after Block 4: anyone hoarse skips their own scream
      block; the `b` performer covers a second set if needed and the
      shortfall is noted in provenance

## 6. Post-processing

Work in a scratch folder, keep the raw session files untouched, and put
finished masters under `artifacts/audio/barks/masters/` (WAV, gitignored;
the repo only receives the `.ogg`).

### 6.1 Cut

Cut the long session file into one WAV per take, named
`<set>-<category>-<nn>-t<k>.wav`. Any editor works. Pick the winning take
per line by ear — listen to all takes of a line back to back, choose the
one that is smallest while still reading, then delete or archive the rest.
Rename the winner `<set>-<category>-<nn>.wav`.

### 6.2 Denoise (only if the room tone is audible at −60 dB)

```bash
# Build a noise profile from the room tone, then apply. Adjust nr (dB) 6–12.
ffmpeg -i room-tone-a.wav -af "afftdn=nr=10:nf=-40:tn=1" -f null - 2>/dev/null
ffmpeg -i a-hurt-heavy-03.wav -af "afftdn=nr=10:nf=-40" a-hurt-heavy-03.dn.wav
```

Skip this step if the tail is already under −60 dBFS RMS; denoising costs
air in the breaths, and the breaths are the point in `flee` and
`hurt-heavy`.

### 6.3 Trim

Remove leading and trailing silence, then fade so no take clicks.

```bash
ffmpeg -i in.wav -af "silenceremove=start_periods=1:start_threshold=-45dB:start_silence=0.01,\
afade=t=in:st=0:d=0.005,\
areverse,silenceremove=start_periods=1:start_threshold=-50dB:start_silence=0.04,\
afade=t=in:st=0:d=0.03,areverse" \
  -ar 48000 -ac 1 -c:a pcm_s24le trimmed.wav
```

(The tail trim and the 30 ms fade-out are done on the reversed signal so
no duration lookup is needed; verified on ffmpeg 8.1.)

### 6.4 Normalise to the category LUFS

Targets from `BARK_SPEC.md` §3: bump/gasp −23, hurt-light/flee/angry/shout
−20, hurt-heavy −18, scream −16. True peak ceiling −1.0 dBTP. Two-pass
`loudnorm` is precise; for sub-2-second files the one-pass linear mode is
fine and avoids the measurement quirk for very short inputs:

```bash
# one-pass, linear gain, category target I=-18 (hurt-heavy shown)
ffmpeg -i trimmed.wav -af "loudnorm=I=-18:TP=-1.0:LRA=7:linear=true:print_format=summary" \
  -ar 48000 -ac 1 -c:a pcm_s24le a-hurt-heavy-03.wav
```

For the very short `bump` and `gasp` files, `loudnorm` may report a
different measured value than expected because the integrated gate needs
≥ 400 ms of signal; if a file is under 0.4 s use peak normalisation
instead: `-af "volume=-6dB"` to taste, aiming for the same perceived level
as its neighbours.

Verify:

```bash
ffmpeg -i a-hurt-heavy-03.wav -af ebur128=peak=true -f null - 2>&1 | grep -E "I:|Peak:"
```

### 6.5 Encode

Ogg/Opus, 48 kHz mono, 40 kb/s VBR. (`BARK_SPEC.md` §3 explains why Opus
over Vorbis; local ffmpeg ships `libopus` only.)

```bash
ffmpeg -i a-hurt-heavy-03.wav -c:a libopus -b:a 40k -vbr on -application audio \
  -frame_duration 20 -ar 48000 -ac 1 a-hurt-heavy-03.ogg
```

Batch, from the masters directory:

```bash
cd artifacts/audio/barks/masters
for f in *.wav; do
  ffmpeg -loglevel error -y -i "$f" -c:a libopus -b:a 40k -vbr on -application audio \
    -frame_duration 20 -ar 48000 -ac 1 "../${f%.wav}.ogg"
done
```

Fallback if a Vorbis file is ever required (needs a `libvorbis` build):

```bash
ffmpeg -i in.wav -c:a libvorbis -q:a 3 -ar 48000 -ac 1 out.ogg
```

### 6.6 Check

```bash
cd artifacts/audio/barks
# format
for f in *.ogg; do ffprobe -v error -select_streams a:0 \
  -show_entries stream=codec_name,sample_rate,channels -of csv=p=0 "$f" | sed "s|^|$f |"; done
# loudness + duration
for f in *.ogg; do printf '%s ' "$f"; ffmpeg -i "$f" -af ebur128 -f null - 2>&1 | grep -oE "I: +[-0-9.]+ LUFS"; done
# total size (must stay comfortably under the 2 MB Phase 3 cap with impacts)
du -ch *.ogg | tail -1
```

Expected: `opus,48000,1` for every file; LUFS within ±1.5 of the category
target; total ≤ ~800 kB.

### 6.7 Manifest and provenance

- Fill `manifest.template.json` → `manifest.json` (one entry per file;
  `pitchOffset` stays 0 in the manifest — the runtime adds the identity
  offset; `gain` is 1.0 unless a specific file needed a nudge after the
  listening pass).
- Add a session block to `artifacts/audio/PROVENANCE.md`: date,
  performers (first name or initials is enough), room, mic, and the
  statement that every file is original.
