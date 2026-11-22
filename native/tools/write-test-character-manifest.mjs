// Generate the runtime manifest from the demo's unmodified public files.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [root] = process.argv.slice(2);
if (!root) throw new Error('usage: node native/tools/write-test-character-manifest.mjs <demo-public-directory>');

const civilian = [
  'civilian-m-01-player', 'civilian-police-m-hero', 'civilian-m-03-hero', 'civilian-f-02-hero',
];
const roster = [
  { id: 'tripo', body: 'characters/tripo-player-ue5.glb', packs: ['tripo'] },
  ...civilian.map(id => ({ id, body: `characters/${id}-ue5.glb`, packs: ['civilian'] })),
  { id: 'motorcycle-rider', body: 'characters/civilian-m-01-player.glb', packs: ['motorcycle'] },
  { id: 'motorcycle-passenger', body: 'characters/civilian-f-02-hero.glb', packs: ['motorcycle'] },
];
const packs = {
  tripo: ['characters/rifle-motion-tripo-v1.glb', 'characters/vehicle-motion-tripo-v1.glb'],
  civilian: ['characters/rifle-motion-ue5-v1.glb', 'characters/vehicle-motion-ue5-v1.glb'],
  motorcycle: ['vehicles/sport_motorcycle_rider_motion.glb'],
};
const motions = Object.values(packs).flat();
const notes = motions.filter(path => path.startsWith('characters/')).map(path => path.replace(/\.glb$/, '.json'));
const files = Object.fromEntries([...new Set([...roster.map(r => r.body), ...motions, ...notes])].sort().map(path => {
  const bytes = readFileSync(join(root, path));
  const asset = { bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
  if (path.endsWith('.glb')) {
    if (bytes.toString('ascii', 0, 4) !== 'glTF' || bytes.readUInt32LE(16) !== 0x4e4f534a) {
      throw new Error(`${path} is not a GLB with a JSON chunk`);
    }
    const json = JSON.parse(bytes.toString('utf8', 20, 20 + bytes.readUInt32LE(12)));
    if (json.animations) asset.clips = json.animations.map(a => a.name);
  }
  return [path, asset];
}));
const manifest = {
  source: 'test/ar_rifle_demo/public',
  provenance: 'characters/SOURCE.md',
  credit: 'Earth Online civilian bodies (MakeHuman/MPFB system assets, CC0-1.0). The Tripo player is user-supplied and for local use only. Animation packs are authored in the source projects.',
  roster,
  packs,
  files,
};
writeFileSync(new URL('../assets/test-characters-manifest.json', import.meta.url), JSON.stringify(manifest, null, 2) + '\n');
