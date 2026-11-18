// write native/assets/rocketbox-motions-manifest.json and
// native/assets/100style-manifest.json from local copies of CodePawl's
// `rocketbox-motions-glb` (objects/ tier: the Microsoft Rocketbox in-place
// clips, MIT) and `100style-glb` (skeleton/ tier, forward walks: The 100STYLE
// Dataset - Ian Mason, CC BY 4.0). Hashes every file the copies hold and
// checks each clip's row in the copy's metadata.jsonl (licence, upstream
// source); nothing in the copies is executed, paths are checked.
//
//   node native/tools/write-motion-manifests.mjs <rocketbox-motions-copy> <hf-revision-sha> <100style-copy> <hf-revision-sha> [out-dir]
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const [rbRoot, rbRev, stRoot, stRev, outArg] = process.argv.slice(2);
const sha40 = /^[0-9a-f]{40}$/;
if (!rbRoot || !stRoot || !sha40.test(rbRev || '') || !sha40.test(stRev || '')) {
  console.error('usage: write-motion-manifests.mjs <rocketbox-motions-copy> <hf-revision-sha> <100style-copy> <hf-revision-sha> [out-dir]');
  process.exit(2);
}
const here = path.dirname(fileURLToPath(import.meta.url));
const outDir = outArg || path.join(here, '..', 'assets');
const safe = (p) => /^[A-Za-z0-9_.-]+(\/[A-Za-z0-9_.-]+)*$/.test(p) && !p.split('/').includes('..');
const sha = (p) => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const rowsOf = (root) => fs.readFileSync(path.join(root, 'metadata.jsonl'), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
const walk = (root, dir) => {
  const out = [];
  for (const e of fs.readdirSync(path.join(root, dir), { withFileTypes: true })) {
    const rel = `${dir}/${e.name}`;
    if (e.isDirectory()) out.push(...walk(root, rel));
    else out.push(rel);
  }
  return out.sort();
};

// ---------------------------------------------------------------- Rocketbox clips
const RB_DATASET = 'codepawl/rocketbox-motions-glb';
const UPSTREAM = 'https://github.com/microsoft/Microsoft-Rocketbox';
const UPSTREAM_COMMIT = '0943055db6ec570bcef9f2c8b41c9e5467c808f9';
const UPSTREAM_PATH = 'Assets/Animations/all_animations_max_motextr_static';
const RB_CREDIT = 'Microsoft Rocketbox Avatar Library (https://github.com/microsoft/Microsoft-Rocketbox), Copyright (c) 2020 Microsoft, MIT License.';
const RB_CHANGES = 'Converted from FBX to GLB in Blender 5.2. Each clip is baked onto the Bip01 skeleton of one Rocketbox avatar (hipoly mesh, 3ds Max materials rebuilt as PBR with colour and normal maps, textures at most 1024 px) and also exported as the bare 121-joint Bip01 skeleton with no mesh. Clip timing and joint rotations are unchanged. Then meshopt-compressed web files, a mobile tier and an uncompressed engine tier.';
{
  const rows = rowsOf(rbRoot);
  const byFile = new Map(rows.map((r) => [r.files && r.files.web, r]));
  const files = [];
  const add = (rel, extra) => {
    if (!safe(rel)) throw new Error('unsafe path ' + rel);
    const abs = path.join(rbRoot, ...rel.split('/'));
    files.push({
      path: rel,
      url: `https://huggingface.co/datasets/${RB_DATASET}/resolve/${rbRev}/${rel}`,
      revision: rbRev,
      bytes: fs.statSync(abs).size,
      sha256: sha(abs),
      licence: 'MIT',
      upstream_commit: UPSTREAM_COMMIT,
      credit: RB_CREDIT,
      ...extra,
    });
  };
  for (const f of ['LICENSES.md', 'README.md', 'metadata.jsonl']) add(f, { what: 'dataset' });
  for (const rel of walk(rbRoot, 'objects')) {
    const r = byFile.get(rel);
    if (!r) throw new Error(`${rel}: not in metadata.jsonl`);
    if (r.license !== 'MIT') throw new Error(`${r.id}: licence ${r.license}`);
    const m = /\/blob\/([0-9a-f]{40})\/(Assets\/.+)$/.exec(r.source_url || '');
    if (!m || m[1] !== UPSTREAM_COMMIT || !m[2].startsWith(UPSTREAM_PATH + '/')) throw new Error(`${r.id}: source ${r.source_url}`);
    add(rel, { clip: r.id, gender: r.gender, seconds: r.seconds, frames: r.frames, fps: r.fps, cast_avatar: r.avatar, upstream_file: m[2] });
  }
  const clips = files.filter((f) => f.clip).length;
  if (clips !== 326) console.warn(`warning: ${clips} clips (the pack has 326)`);
  const manifest = {
    schema: 'earthonline-motion-manifest-v1',
    source: {
      dataset: `https://huggingface.co/datasets/${RB_DATASET}`,
      revision: rbRev,
      tier: 'objects',
      packer: 'CodePawl (https://huggingface.co/codepawl)',
      upstream: UPSTREAM,
      upstream_commit: UPSTREAM_COMMIT,
      upstream_path: UPSTREAM_PATH,
      licence: 'MIT',
      licence_text: 'LICENSES.md (travels with every copy); upstream LICENSE.md at the commit above: MIT License, Copyright (c) 2020 Microsoft',
    },
    credit: RB_CREDIT,
    changes_made: RB_CHANGES,
    local_changes: 'Cooked at load by native/eo-agents/src/motion.rs and rocketbox.rs: windows of the standing clips (motion::ROLES) played on each Rocketbox avatar\'s own Bip01 nodes (rotations; the pelvis travel scaled by the root height), the seam cross-faded, sampled into the baked joint-matrix tables. The downloaded files are not modified.',
    files,
  };
  const out = path.join(outDir, 'rocketbox-motions-manifest.json');
  fs.writeFileSync(out, JSON.stringify(manifest, null, 1) + '\n');
  console.log(`wrote ${out}: ${files.length} files (${clips} clips), ${(files.reduce((a, f) => a + f.bytes, 0) / 1e6).toFixed(1)} MB`);
}

// ---------------------------------------------------------------- 100STYLE walks
const ST_DATASET = 'codepawl/100style-glb';
const ST_CREDIT = 'The 100STYLE Dataset - Ian Mason';
const ST_CHANGES = "Trimmed with the dataset's Frame_Cuts.csv, converted from centimetres to metres, exported as glTF 2.0; the source skeleton clips keep the original joint rotations and the root motion.";
{
  const rows = rowsOf(stRoot);
  const byFile = new Map(rows.map((r) => [r.files && r.files.skeleton, r]));
  const files = [];
  const add = (rel, extra) => {
    if (!safe(rel)) throw new Error('unsafe path ' + rel);
    const abs = path.join(stRoot, ...rel.split('/'));
    files.push({
      path: rel,
      url: `https://huggingface.co/datasets/${ST_DATASET}/resolve/${stRev}/${rel}`,
      revision: stRev,
      bytes: fs.statSync(abs).size,
      sha256: sha(abs),
      licence: 'CC-BY-4.0',
      credit: ST_CREDIT,
      ...extra,
    });
  };
  for (const f of ['LICENSES.md', 'README.md', 'metadata.jsonl']) add(f, { what: 'dataset' });
  for (const rel of walk(stRoot, 'skeleton')) {
    const r = byFile.get(rel);
    if (!r) throw new Error(`${rel}: not in metadata.jsonl`);
    if (r.movement !== 'FW') throw new Error(`${rel}: ${r.movement} is not a forward walk`);
    add(rel, { clip: r.id, style: r.style, description: r.description, seconds: r.seconds, frames: r.frames, fps: r.fps });
  }
  const manifest = {
    schema: 'earthonline-motion-manifest-v1',
    source: {
      dataset: `https://huggingface.co/datasets/${ST_DATASET}`,
      revision: stRev,
      tier: 'skeleton',
      packer: 'CodePawl (https://huggingface.co/codepawl)',
      upstream: 'https://www.ianxmason.com/100style/',
      upstream_record: 'https://zenodo.org/records/8127870',
      upstream_record_licence: 'cc-by-4.0 (Zenodo API, record published 2022-05-04, 100STYLE.zip 1468515354 bytes)',
      licence: 'CC-BY-4.0',
      licence_text: 'LICENSES.md (travels with every copy): CC-BY-4.0, https://creativecommons.org/licenses/by/4.0/',
      paper: 'Ian Mason, Sebastian Starke, Taku Komura. Real-Time Style Modelling of Human Locomotion via Feature-Wise Transformations and Local Motion Phases. Proc. ACM Comput. Graph. Interact. Tech. 5(1), 2022.',
    },
    credit: ST_CREDIT,
    changes_made: ST_CHANGES,
    local_changes: 'Cooked at load by native/eo-agents/src/motion.rs and rocketbox.rs: one gait cycle cut from each forward walk, made in place and looped, retargeted onto the Rocketbox Bip01 skeletons (world rotation changes from the source T-pose; hips travel scaled by the leg length; relaxed fingers from a Rocketbox idle), sampled into the baked joint-matrix tables. The downloaded files are not modified.',
    files,
  };
  const out = path.join(outDir, '100style-manifest.json');
  fs.writeFileSync(out, JSON.stringify(manifest, null, 1) + '\n');
  console.log(`wrote ${out}: ${files.length} files, ${(files.reduce((a, f) => a + f.bytes, 0) / 1e6).toFixed(1)} MB`);
}
