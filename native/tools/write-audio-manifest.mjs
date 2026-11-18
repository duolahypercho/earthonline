// The sound bank's provenance and the game's clip list, from the downloads and
// the automatic screen (nobody has listened to these clips yet):
//
//   <root>/MANIFEST.tsv            one line per downloaded clip: source, Freesound ID, uploader,
//                                  licence, source URL, size, SHA-256, screen verdict
//   <root>/REVIEW.md               the 30 clips the game uses most, to listen through first
//   native/assets/audio-manifest.json   the clips eo-audio loads (verified by size and SHA-256)
//   native/assets/AUDIO-CREDITS.md      every CC BY clip downloaded (uploader, Freesound ID, title)
//
// Inputs under <root> (the argument, or EO_AUDIO_ROOT): fsd50k/selection.json and
// fsd50k/fetch-log.json (scripts that fetched them; per-clip licence from the
// Zenodo 4060432 metadata), sfx-kit/ (wyzowl/sfx-kit) with sfx-kit/fetch-log.json,
// and screen.json (eo-audio/examples/screen.rs).
//   node native/tools/write-audio-manifest.mjs [root]
import fs from 'node:fs';
import path from 'node:path';
import url from 'node:url';

const native = path.resolve(path.dirname(url.fileURLToPath(import.meta.url)), '..');
const root = process.argv[2] || process.env.EO_AUDIO_ROOT || (() => { console.error('usage: write-audio-manifest.mjs <root> (or set EO_AUDIO_ROOT)'); process.exit(2); })();
const read = (p) => JSON.parse(fs.readFileSync(path.join(root, p), 'utf8'));
const sel = read('fsd50k/selection.json');
const fsdLog = read('fsd50k/fetch-log.json');
const kitLog = read('sfx-kit/fetch-log.json');
// JSON has no -inf or NaN: the screen writes null for a silent clip's loudness or a short clip's spread.
const screen = new Map(read('screen.json').map((s) => [s.path, { ...s, lufs: s.lufs ?? -Infinity, spread_db: s.spread_db ?? Infinity }]));
const HF_FSD = 'Fhrozen/FSD50k@ccf1acaff12f3f4a4c10052dddafbb6d22152c9b';
const HF_KIT = 'wyzowl/sfx-kit@74e7a19b22f9f99f1d44a7b42f3b843aa18b24ab';
const LIC_URL = { 'CC0-1.0': 'https://creativecommons.org/publicdomain/zero/1.0/', 'CC-BY-3.0': 'https://creativecommons.org/licenses/by/3.0/', 'CC-BY-4.0': 'https://creativecommons.org/licenses/by/4.0/' };

// ---- the screen's verdict per clip (format, clipping, loudness, noise, DC, silence)
// Events that must stand out of their recording's background (a step, a honk, a door);
// birds a little less. Continuous sounds (sirens, foghorns, squeal, beds) have no such test:
// their 'floor' is the sound itself. The class is the clip's first role.
const EVENT = /^(footsteps|horn|brakes|car_door|door|glass)/;
const BIRD = /^birds/;
function verdict(s, roles) {
  if (!s || s.error) return ['drop', `unreadable: ${s ? s.error : 'not screened'}`];
  if (s.rate < 22050) return ['drop', `low rate: ${s.rate} Hz`];
  if (s.clipped_frac > 0.001 || s.clip_runs > 25) return ['drop', `clipped (${(s.clipped_frac * 100).toFixed(2)} % of samples, ${s.clip_runs} runs)`];
  if (Math.abs(s.dc) > 0.02) return ['drop', `DC offset ${s.dc.toFixed(3)}`];
  if (roles[0] === 'ui') {
    // Interface clicks are shorter than a loudness block: judged by their peak.
    return s.peak_db > -40 ? ['pass', ''] : ['drop', `too quiet (peak ${s.peak_db.toFixed(1)} dBFS)`];
  }
  if (s.duration_s < 0.25) return ['drop', `too short: ${s.duration_s.toFixed(2)} s`];
  if (!(s.lufs > -45)) return ['drop', `too quiet (${s.lufs.toFixed(1)} LUFS)`];
  if (s.lufs > -3) return ['drop', `too hot (${s.lufs.toFixed(1)} LUFS)`];
  const snr = s.loud_db - s.floor_db;
  const event = EVENT.test(roles[0]);
  if (!event && s.silent_frac > 0.6) return ['drop', `mostly silent: ${(s.silent_frac * 100).toFixed(0)} %`];
  if (event && snr < 15) return ['drop', `noisy: ${snr.toFixed(1)} dB over its floor`];
  if (BIRD.test(roles[0]) && snr < 8) return ['drop', `noisy: ${snr.toFixed(1)} dB over its floor`];
  return ['pass', ''];
}

// The clips the screen dropped were deleted from the store (user, 2026-11-15), so
// the manifest and the credits list only the files still present.
const present = (x) => fs.existsSync(path.join(root, x.rel));
const fsd = sel.clips.map((c) => {
  const rel = `fsd50k/clips/${c.split}/${c.fname}.wav`;
  const log = fsdLog[`clips/${c.split}/${c.fname}.wav`];
  const s = screen.get(rel);
  const [v, why] = verdict(s, c.roles);
  return { kind: 'fsd', c, rel, log, s, v, why, roles: c.roles, text: `${c.title} ${c.tags.join(' ')}` };
}).filter(present);
const kitMeta = new Map(fs.readFileSync(path.join(root, 'sfx-kit/metadata.jsonl'), 'utf8').split(/\r?\n/).filter(Boolean).map((l) => JSON.parse(l)).map((m) => [m.file_name, m]));
const kit = Object.entries(kitLog).filter(([p, l]) => l.ok && p.endsWith('.wav')).map(([p, log]) => {
  const rel = `sfx-kit/${p}`;
  const s = screen.get(rel);
  const [v, why] = verdict(s, ['ui']);
  return { kind: 'kit', rel, log, s, v, why, roles: ['ui'], meta: kitMeta.get(p) };
}).filter(present);

// ---- MANIFEST.tsv
const tsv = ['source\tfreesound_id\tuploader\tlicence\tsource_url\tfetched_from\tbytes\tsha256\tpath\troles\tscreen\tscreen_note'];
for (const x of fsd) tsv.push(['FSD50K', x.c.fname, x.c.uploader, x.c.licence, `https://freesound.org/s/${x.c.fname}/`, x.log.url, x.log.size, x.log.sha256, x.rel, x.roles.join(','), x.v, x.why].join('\t'));
for (const x of kit) tsv.push(['wyzowl/sfx-kit', '', 'Wyzowl', 'CC0-1.0', 'https://github.com/wyzowl/sfx-kit', x.log.url, x.log.size, x.log.sha256, x.rel, 'ui', x.v, x.why].join('\t'));
fs.writeFileSync(path.join(root, 'MANIFEST.tsv'), tsv.join('\n') + '\n');

// ---- the game's bank
const pass = fsd.filter((x) => x.v === 'pass' && !x.c.not_for_game);
const has = (x, r) => x.roles.includes(r);
const steady = (x) => x.s.spread_db;
const snr = (x) => x.s.loud_db - x.s.floor_db;
const cc0First = (a, b) => (a.c.licence === 'CC0-1.0' ? 0 : 1) - (b.c.licence === 'CC0-1.0' ? 0 : 1);
const take = (list, n, order) => [...list].sort(order).slice(0, n);
const used = new Map(); // rel -> entry
const bank = [];
const TARGET = { bed: -28, loop: -24, shot: -22, step: null, far: -24, ui: -26 };
function add(x, role, group, extra = {}) {
  const s = x.s;
  // Steps and interface ticks are shorter than a loudness block: matched by their peak.
  let gain = group === 'step' ? -6 - s.peak_db : group === 'ui' ? -12 - s.peak_db : TARGET[group] - s.lufs;
  gain = Math.max(-18, Math.min(18, gain, -1 - s.peak_db));
  const playedThrough = ['bed', 'far'].includes(group);
  const e = {
    id: x.kind === 'fsd' ? `fsd-${x.c.fname}-${role}` : `kit-${path.basename(x.rel, '.wav')}-${role}`,
    path: x.rel, bytes: x.log.size, sha256: x.log.sha256, role, source: x.kind === 'fsd' ? 'FSD50K' : 'wyzowl/sfx-kit',
    freesound_id: x.kind === 'fsd' ? Number(x.c.fname) : null, uploader: x.kind === 'fsd' ? x.c.uploader : 'Wyzowl',
    licence: x.kind === 'fsd' ? x.c.licence : 'CC0-1.0', title: x.kind === 'fsd' ? x.c.title : x.meta?.name ?? path.basename(x.rel),
    rate: s.rate, channels: s.channels, frames: s.frames, lufs: Number.isFinite(s.lufs) ? +s.lufs.toFixed(2) : -99, peak_db: +s.peak_db.toFixed(2), gain_db: +gain.toFixed(2),
    stream: playedThrough && s.duration_s > 8, ...extra,
  };
  bank.push(e);
  used.set(x.rel, (used.get(x.rel) || []).concat(role));
}
const isSF = (x) => !!x.c.sf || /san francisco|bay area/i.test(`${x.c.title} ${x.c.description}`);
function loopOf(x) {
  const w = x.s.steady;
  if (!w) return null;
  const xf = Math.round(0.25 * x.s.rate);
  const ls = w.start + xf, le = w.start + w.len;
  if (le - ls < 2 * xf) return null;
  return { loop: [ls, le, xf], decode_frames: Math.min(x.s.frames, le + 16) };
}
// A bed recorded in San Francisco is taken a little less steady (4 dB more spread) and first.
const bed = (pool, n, min_s, max_spread) => take(pool.filter((x) => x.s.duration_s >= min_s && steady(x) <= max_spread + (isSF(x) ? 4 : 0)), n, (a, b) => (isSF(b) - isSF(a)) || steady(a) - steady(b) || b.s.duration_s - a.s.duration_s);
const quietWords = /\b(distant|far|night|quiet|residential|suburb|suburban|calm|background|neighbou?rhood|evening|morning|park)\b/i;
const indoorRain = /\b(roof|tin|umbrella|window|car|tent|indoor|inside|gutter|drain|attic)\b/i;
for (const x of bed(pass.filter((x) => has(x, 'city_ambience') || has(x, 'traffic')), 16, 15, 6)) add(x, 'bed_city', 'bed');
for (const x of bed(pass.filter((x) => (has(x, 'city_ambience') || has(x, 'traffic') || has(x, 'birds_city')) && quietWords.test(x.text) && !used.has(x.rel)), 8, 15, 5)) add(x, 'bed_quiet', 'bed');
for (const x of bed(pass.filter((x) => has(x, 'crowd_walla')), 8, 8, 6)) add(x, 'bed_walla', 'bed');
for (const x of bed(pass.filter((x) => has(x, 'rain') && !indoorRain.test(x.text)), 10, 15, 4)) add(x, 'bed_rain', 'bed');
for (const x of bed(pass.filter((x) => has(x, 'wind') && !/\b(microphone|mic|howl|howling|storm|hurricane|tunnel)\b/i.test(x.text)), 8, 12, 7)) add(x, 'bed_wind', 'bed');
for (const x of bed(pass.filter((x) => has(x, 'sf_waterfront_water') || (has(x, 'birds_gull') && /\b(waves?|lapping|shore|harbou?r|marina|sea)\b/i.test(x.text))), 5, 10, 7)) add(x, 'bed_water', 'bed');
const loops = (pool, n, max) => take(pool.map((x) => [x, loopOf(x)]).filter(([x, l]) => l && x.s.steady.spread_db <= max), n, ([a], [b]) => a.s.steady.spread_db - b.s.steady.spread_db);
const notHeavy = (x) => !x.c.labels.some((l) => ['Bus', 'Truck'].includes(l));
for (const [x, l] of loops(pass.filter((x) => has(x, 'engine_idle') && notHeavy(x)), 10, 1.5)) add(x, 'engine_car', 'loop', l);
for (const [x, l] of loops(pass.filter((x) => (has(x, 'engine_bus') || has(x, 'engine_truck')) && x.c.labels.includes('Idling')), 6, 2.0)) add(x, 'engine_heavy', 'loop', l);
for (const [x, l] of loops(pass.filter((x) => has(x, 'engine_idle') && notHeavy(x) && x.c.licence === 'CC0-1.0'), 2, 1.2)) add(x, 'engine_player_lo', 'loop', l);
for (const [x, l] of loops(pass.filter((x) => has(x, 'engine_accel') && notHeavy(x)), 3, 2.0)) add(x, 'engine_player_hi', 'loop', l);
for (const [x, l] of loops(pass.filter((x) => has(x, 'tyre_squeal') && x.s.duration_s >= 1.5), 3, 3.0)) add(x, 'tyre_squeal', 'loop', l);
for (const [x, l] of loops(pass.filter((x) => has(x, 'sf_cable_car')), 1, 99)) add(x, 'cable_slot', 'loop', l);
const shots = (pool, n, max_s, min_snr = 15) => take(pool.filter((x) => x.s.duration_s <= max_s && snr(x) >= min_snr), n, (a, b) => (isSF(b) - isSF(a)) || cc0First(a, b) || snr(b) - snr(a));
for (const x of shots(pass.filter((x) => has(x, 'brakes')), 6, 6)) add(x, 'brake', 'shot');
for (const x of shots(pass.filter((x) => has(x, 'horn')), 14, 3.5)) add(x, 'horn', 'shot');
for (const x of shots(pass.filter((x) => has(x, 'car_door')), 8, 3.0)) add(x, 'car_door', 'shot');
for (const x of shots(pass.filter((x) => has(x, 'birds_gull') && !/\b(waves?|lapping)\b/i.test(x.text)), 16, 8, 8)) add(x, 'gull', 'shot');
for (const x of shots(pass.filter((x) => has(x, 'birds_pigeon')), 8, 8, 8)) add(x, 'pigeon', 'shot');
for (const x of shots(pass.filter((x) => has(x, 'birds_city')), 12, 8, 8)) add(x, 'bird', 'shot');
for (const x of take(pass.filter((x) => has(x, 'thunder') && x.s.duration_s >= 4 && x.s.duration_s <= 20), 8, (a, b) => snr(b) - snr(a))) add(x, 'thunder', 'far');
for (const x of pass.filter((x) => has(x, 'siren_us') && x.s.duration_s >= 4)) add(x, 'siren', 'far');
// The SF set: every clip that passed, in a set order (the foghorn stations take the first
// three in turn: the Golden Gate Bridge the San Francisco recording, Alcatraz and Yerba Buena
// Island the cleanest other foghorns). Read and left out: 38017 (on a boat in fog: waves,
// gulls and a ship's bell over the horn) and 14577 (footsteps on Muni station stairs, not a train).
const SF_ORDER = { foghorn: ['99630', '371290', '234681', '37915', '128093'], subway: ['38731', '38732'] };
for (const r of [['sf_foghorn', 'foghorn'], ['sf_ship_horn', 'ship_horn'], ['sf_streetcar', 'streetcar'], ['sf_transit_under', 'subway']]) {
  const order = SF_ORDER[r[1]];
  const pool = pass.filter((x) => x.c.roles[0] === r[0] && (!order || order.includes(x.c.fname))).sort((a, b) => (order ? order.indexOf(a.c.fname) - order.indexOf(b.c.fname) : Number(a.c.fname) - Number(b.c.fname)));
  for (const x of pool) add(x, r[1], 'far');
}
// Steps: clips with at least four slices.
for (const [r, g] of [['footsteps_concrete', 'foot_concrete'], ['footsteps_asphalt', 'foot_asphalt'], ['footsteps_wood', 'foot_wood'], ['footsteps_metal', 'foot_metal'], ['footsteps_gravel', 'foot_gravel'], ['footsteps_generic', 'foot_generic'], ['footsteps_wet', 'foot_wet']]) {
  const pool = pass.filter((x) => has(x, r) && (x.s.slices || []).length >= 4);
  for (const x of take(pool, 10, (a, b) => snr(b) - snr(a))) {
    const slices = x.s.slices.slice(0, 16);
    // Decode only as far as the last step used.
    add(x, g, 'step', { slices, decode_frames: Math.min(x.s.frames, Math.max(...slices.map((q) => q[0] + q[1])) + 16) });
  }
}
// The interface (sfx-kit).
const UI = {
  ui_hover: ['interface/click-hover-soft.wav', 'interface/click-hover-medium.wav'],
  ui_move: ['interface/click-tap-soft.wav', 'interface/click-tap-medium.wav'],
  ui_confirm: ['interface/click-button-medium.wav', 'interface/click-button-firm.wav'],
  ui_back: ['interface/click-latch-soft.wav'],
  ui_toggle: ['interface/click-toggle-on-medium.wav', 'interface/click-toggle-off-medium.wav'],
  ui_start: ['notifications/notification-success-warm.wav'],
  ui_open: ['whooshes/whoosh-fast-soft.wav'],
};
for (const [r, files] of Object.entries(UI)) for (const f of files) {
  const x = kit.find((k) => k.rel === `sfx-kit/sounds/${f}`);
  if (x && x.v === 'pass') add(x, r, 'ui');
}
// Kept in the order chosen: within a role, the first clip is the preferred one (a foghorn station's own voice).
const manifest = {
  version: 1,
  generated_by: 'native/tools/write-audio-manifest.mjs',
  note: 'Chosen by an automatic screen only. Nobody has listened to these clips yet (see REVIEW.md in the audio root).',
  root_env: 'EO_AUDIO_ROOT',
  sources: { fsd50k: `https://zenodo.org/records/4060432 (metadata), audio from https://huggingface.co/datasets/${HF_FSD}`, sfx_kit: `https://huggingface.co/datasets/${HF_KIT}` },
  stream_over_s: 8.0,
  clips: bank,
};
fs.writeFileSync(path.join(native, 'assets/audio-manifest.json'), JSON.stringify(manifest, null, 1) + '\n');

// ---- credits: every CC BY clip downloaded
const by = fsd.filter((x) => x.c.licence.startsWith('CC-BY')).sort((a, b) => a.c.uploader.localeCompare(b.c.uploader, 'en') || Number(a.c.fname) - Number(b.c.fname));
const esc = (t) => t.replace(/[|\\`*_[\]<>]/g, (m) => '\\' + m).replace(/\s+/g, ' ').trim();
const credits = [
  '# Sound credits',
  '',
  'Earth Online\'s recorded sounds come from two collections. Generated by `native/tools/write-audio-manifest.mjs`; do not edit by hand.',
  '',
  '- **FSD50K** (Eduardo Fonseca, Xavier Favory, Jordi Pons, Frederic Font, Xavier Serra, "FSD50K: an Open Dataset of Human-Labeled Sound Events", 2020; https://zenodo.org/records/4060432), released under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Each clip is a Freesound recording under its own licence; only CC0 and CC BY clips are used. Audio was taken from the mirror `' + HF_FSD + '` and checked against the upstream clip IDs and the mirror\'s SHA-256.',
  '- **SFX Kit by Wyzowl** (https://github.com/wyzowl/sfx-kit), CC0 1.0: the interface sounds.',
  '',
  'Changes: the game plays these clips trimmed (single footsteps cut from walking recordings, loops taken from engine recordings), mixed to mono, level-matched, pitched and filtered in its 3D mixer. CC0 clips need no credit; their uploaders are listed in `MANIFEST.tsv` beside the audio.',
  '',
  `## CC BY clips (${by.length}; marked * when the game plays it)`,
  '',
  '| Uploader | Freesound ID | Title | Licence |',
  '| --- | --: | --- | --- |',
  ...by.map((x) => `| ${esc(x.c.uploader)} | [${x.c.fname}](https://freesound.org/s/${x.c.fname}/)${used.has(x.rel) ? ' *' : ''} | ${esc(x.c.title)} | [${x.c.licence.replace(/-/g, ' ').replace('CC BY', 'CC BY')}](${LIC_URL[x.c.licence]}) |`),
  '',
];
fs.writeFileSync(path.join(native, 'assets/AUDIO-CREDITS.md'), credits.join('\n'));

// ---- the review list: what the game plays most, first
const order = [['foot_concrete', 3], ['foot_asphalt', 2], ['foot_generic', 1], ['engine_car', 4], ['engine_heavy', 1], ['bed_city', 3], ['bed_quiet', 2], ['horn', 2], ['engine_player_lo', 1], ['engine_player_hi', 1], ['bed_rain', 1], ['bed_walla', 1], ['gull', 1], ['siren', 1], ['foghorn', 1], ['streetcar', 1], ['cable_slot', 1], ['tyre_squeal', 1], ['ui_hover', 1], ['ui_confirm', 1]];
const why = { foot_concrete: 'every step on a sidewalk', foot_asphalt: 'every step on the roadway', foot_generic: 'steps where no surface pool fits', engine_car: 'every sedan and taxi near you, looped and pitched by speed', engine_heavy: 'every truck and bus', bed_city: 'the downtown bed under everything', bed_quiet: 'the residential bed', horn: 'horns at jams', engine_player_lo: 'your car, idle to mid revs', engine_player_hi: 'your car, high revs', bed_rain: 'rain', bed_walla: 'crowded sidewalks', gull: 'the waterfront', siren: 'distant sirens', foghorn: 'the bay in fog', streetcar: 'streetcars on Market Street', cable_slot: 'along the cable car lines', tyre_squeal: 'your car sliding', ui_hover: 'menu hover', ui_confirm: 'menu select' };
const review = ['# Listen to these first', '', 'Nobody has listened to any clip yet: the game chose them by an automatic screen (length, clipping, loudness, noise floor, DC, steadiness). These 30 are the ones a player hears most; listen in this order and strike any that sound wrong (`native/tools/write-audio-manifest.mjs` takes an exclusion list).', '', '| # | Role (what it is for) | File | Freesound | What to listen for |', '| --: | --- | --- | --- | --- |'];
let n = 0;
for (const [r, k] of order) {
  for (const e of bank.filter((b) => b.role === r).slice(0, k)) {
    if (n >= 30) break;
    n++;
    const what = r.startsWith('foot') ? 'single steps: a clean heel-toe, no scuffs or voices; the slices land on the steps' : r.startsWith('engine') || r === 'tyre_squeal' || r === 'cable_slot' ? 'the loop (seconds ' + (e.loop ? (e.loop[0] / e.rate).toFixed(1) + '-' + (e.loop[1] / e.rate).toFixed(1) : '?') + '): steady, no gear change, no voices' : r.startsWith('bed') ? 'no close voices, music, alarms or a dominant single event' : r.startsWith('ui') ? 'a short, quiet tick that sits under the menus (made by code, CC0)' : 'it is what it says, recorded (not synthesised), no voices';
    review.push(`| ${n} | ${r} (${why[r]}) | \`${e.path}\` | ${e.freesound_id ? `[${e.freesound_id}](https://freesound.org/s/${e.freesound_id}/)` : 'sfx-kit'} | ${what} |`);
  }
}
fs.writeFileSync(path.join(root, 'REVIEW.md'), review.join('\n') + '\n');

// ---- summary
const roles = {};
for (const e of bank) roles[e.role] = (roles[e.role] || 0) + 1;
const verdicts = { pass: 0, drop: 0 };
const reasons = {};
for (const x of [...fsd, ...kit]) {
  verdicts[x.v]++;
  if (x.v === 'drop') {
    const k = x.why.split(/[:(]/)[0].trim();
    reasons[k] = (reasons[k] || 0) + 1;
  }
}
const lic = {};
for (const e of bank) lic[e.licence] = (lic[e.licence] || 0) + 1;
console.log(JSON.stringify({ downloaded: fsd.length + kit.length, verdicts, reasons, bank: bank.length, roles, licences_in_bank: lic, cc_by_credited: by.length, review: n }, null, 1));
