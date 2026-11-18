// write native/assets/people-manifest.json from a local copy of the
// Rocketbox people (CodePawl's GLB repackaging of the Microsoft Rocketbox
// Avatar Library, MIT). Reads the copy's metadata.jsonl and hashes every file
// it lists for the humans (objects/ and lods/), plus LICENSES.md, README.md
// and metadata.jsonl. Nothing in the copy is executed; paths are checked.
//
//   node native/tools/write-people-manifest.mjs <copy-root> <hf-revision-sha> [out.json]
//
// Roles: `civilian` (the random street pool), `child`, `uniform:<role>` (kept
// for later roles), `reserve:<why>`. The pool order interleaves women and men.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const [root, revision, outArg] = process.argv.slice(2);
if (!root || !/^[0-9a-f]{40}$/.test(revision || '')) {
  console.error('usage: write-people-manifest.mjs <copy-root> <hf-revision-sha> [out.json]');
  process.exit(2);
}
const here = path.dirname(fileURLToPath(import.meta.url));
const out = outArg || path.join(here, '..', 'assets', 'people-manifest.json');
const DATASET = 'codepawl/rocketbox-avatars-glb';
const UPSTREAM = 'https://github.com/microsoft/Microsoft-Rocketbox';
const UPSTREAM_COMMIT = '0943055db6ec570bcef9f2c8b41c9e5467c808f9';
const CREDIT = 'Microsoft Rocketbox Avatar Library (https://github.com/microsoft/Microsoft-Rocketbox), Copyright (c) 2020 Microsoft, MIT License.';
const CHANGES = 'Converted from FBX and TGA to GLB and PNG in Blender 5.2: the highest-detail mesh kept, the 3ds Max materials rebuilt as PBR (colour and normal maps; specular maps left out), hair and eyelashes set to alpha cutout, the one-frame bind clip dropped. Rig (Bip01 skeleton) and face blendshapes kept. Then meshopt-compressed web files, KTX2 textures, LODs, a mobile tier, an uncompressed engine tier and convex collision hulls.';

const safe = (p) => /^[A-Za-z0-9_.-]+(\/[A-Za-z0-9_.-]+)*$/.test(p) && !p.split('/').includes('..');
const sha = (p) => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const rows = fs.readFileSync(path.join(root, 'metadata.jsonl'), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));

// Who walks the street. Uniforms (and work wear that reads as one) are kept
// for later roles; children are out of the crowd until the owner decides.
// wood-male-01 (denim shirt and jeans, no gear) reads as everyday clothes: in the pool.
const UNIFORM = [
  ['police-', 'police'], ['fire-', 'fire'], ['military-', 'military'], ['medical-', 'medical'], ['pilot-', 'pilot'],
  ['security-', 'security'], ['chef-', 'chef'], ['construction-', 'construction'], ['delivery-', 'delivery'],
  ['gardener-', 'gardener'], ['sports-', 'sports-kit'],
];
const RESERVE = { 'female-adult-16': 'older-low-detail-model', 'female-party-01': 'evening-wear', 'female-party-02': 'evening-wear' };
function roleOf(r) {
  if (r.kind === 'animal') return 'animal';
  if (r.kind === 'child') return 'child';
  if (RESERVE[r.id]) return 'reserve:' + RESERVE[r.id];
  for (const [p, role] of UNIFORM) if (r.id.startsWith(p)) return 'uniform:' + role;
  return 'civilian';
}

const files = [];
const addFile = (rel, extra) => {
  if (!safe(rel)) throw new Error('unsafe path ' + rel);
  const abs = path.join(root, ...rel.split('/'));
  const st = fs.statSync(abs);
  files.push({
    path: rel,
    url: `https://huggingface.co/datasets/${DATASET}/resolve/${revision}/${rel}`,
    revision,
    bytes: st.size,
    sha256: sha(abs),
    licence: 'MIT',
    upstream_commit: UPSTREAM_COMMIT,
    credit: CREDIT,
    changes_made: CHANGES,
    ...extra,
  });
};
for (const f of ['LICENSES.md', 'README.md', 'metadata.jsonl']) addFile(f, { what: 'dataset' });
const avatars = [];
for (const r of rows) {
  if (r.kind === 'animal') continue;
  if (r.license !== 'MIT') throw new Error(`${r.id}: licence ${r.license}`);
  const m = /\/tree\/([0-9a-f]{40})\/(Assets\/.+)$/.exec(r.source_url || '');
  if (!m || m[1] !== UPSTREAM_COMMIT) throw new Error(`${r.id}: source ${r.source_url}`);
  const tiers = {};
  for (const k of ['web', 'lod1', 'lod2', 'lod3']) {
    tiers[k] = r.files[k];
    addFile(r.files[k], { avatar: r.id, tier: k });
  }
  const role = roleOf(r);
  avatars.push({ id: r.id, title: r.title, kind: r.kind, group: r.group, upstream_path: m[2], role, pool: role === 'civilian', triangles: r.triangles, bones: r.bones, files: tiers });
}
// Pool order: women and men interleaved; within each, the everyday adults and
// the business wear mixed in proportion, so a pool cut short by the renderer's
// mesh budget keeps the same mix.
const civ = avatars.filter((a) => a.pool);
const mix = (list) => {
  const adults = list.filter((a) => a.kind === 'adult').sort((a, b) => a.id.localeCompare(b.id));
  const work = list.filter((a) => a.kind !== 'adult').sort((a, b) => a.id.localeCompare(b.id));
  const out = [];
  let i = 0;
  let j = 0;
  while (i < adults.length || j < work.length) {
    // Take whichever list is further behind its share.
    if (j >= work.length || (i < adults.length && i / adults.length <= j / work.length)) out.push(adults[i++]);
    else out.push(work[j++]);
  }
  return out;
};
const women = mix(civ.filter((a) => a.id.includes('female')));
const men = mix(civ.filter((a) => !a.id.includes('female')));
const ordered = [];
for (let i = 0; i < Math.max(women.length, men.length); i++) {
  if (women[i]) ordered.push(women[i]);
  if (men[i]) ordered.push(men[i]);
}
const rest = avatars.filter((a) => !a.pool);
const manifest = {
  schema: 'earthonline-people-manifest-v1',
  source: {
    dataset: `https://huggingface.co/datasets/${DATASET}`,
    revision,
    packer: 'CodePawl (https://huggingface.co/codepawl)',
    upstream: UPSTREAM,
    upstream_commit: UPSTREAM_COMMIT,
    licence: 'MIT',
    licence_text: 'LICENSES.md (travels with every copy); upstream LICENSE.md at the commit above: MIT License, Copyright (c) 2020 Microsoft',
  },
  credit: CREDIT,
  changes_made: CHANGES,
  local_changes: 'Cooked at load by native/eo-agents/src/rocketbox.rs: blendshapes dropped, tiers mapped (full mesh hero, lod1 near, lod2 mid, lod3 far), materials packed into one 1024 px BC atlas per avatar with roughness by material class, the face rig folded into the head, moved by the Rocketbox clips and the 100STYLE walks (motion.rs). The downloaded files are not modified.',
  files,
  avatars: [...ordered, ...rest],
};
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, JSON.stringify(manifest, null, 1) + '\n');
const bytes = files.reduce((a, f) => a + f.bytes, 0);
console.log(`wrote ${out}: ${files.length} files, ${(bytes / 1e6).toFixed(1)} MB; ${avatars.length} people, ${ordered.length} in the pool`);
for (const [k, v] of Object.entries(avatars.reduce((m, a) => ((m[a.role] = (m[a.role] || 0) + 1), m), {}))) console.log(`  ${k}: ${v}`);
