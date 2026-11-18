/**
 * The canonical metric rest skeleton every authored humanoid binds to.
 *
 * `character-asset-contract.mjs` owns the *vocabulary* (which bones exist and
 * who parents whom). It deliberately carries no measurements. This module adds
 * the missing half: where each of those joints actually sits, in metres, for
 * the reference adult.
 *
 * The measurements are not invented here. `REST_POSE` in
 * `src/simulation/pedestrians/pedestrian-presentation.js` already pins 17 of
 * these joints, and the runtime's authored locomotion clips, the two-bone IK
 * (`LEG_SEGMENTS`) and the foot-plant solver (`GAIT.soleOffset`,
 * `GAIT.stanceHalfWidth`) are all written against those exact numbers. This
 * skeleton reproduces them bit-for-bit and fills in the 39 joints the runtime
 * does not yet drive (spine break, shoulders, jaw, eyes, fingers, toes).
 *
 * Editing `REST_POSE` would let an authored body drift away from the
 * animation that has to play on it.
 *
 * The governing rule for the whole character pipeline:
 *
 *     Conform the mesh to this skeleton. Never this skeleton to the mesh.
 *
 * Moving a joint here invalidates `LEG_SEGMENTS`, `GAIT.legRatio`,
 * `GAIT.soleOffset`, the IK, and every authored clip value. Non-uniformly
 * scaling a source body's limbs in a DCC is free; re-authoring the animation
 * system is not.
 *
 * ---------------------------------------------------------------------------
 * COORDINATE FRAME  (verified against runtime code, not assumed)
 * ---------------------------------------------------------------------------
 *   +Y   up
 *   +Z   the character's FORWARD   (`heading = 0` gives fwd = (sin0, cos0) = +Z)
 *   +X   the character's LEFT      (`left = (cos0, -sin0)` = +X; and `REST_POSE`
 *                                   puts `LeftArm` at +0.185 X)
 *   right-handed, 1 unit = 1 metre, root pivot on the ground between the feet.
 *
 * ---------------------------------------------------------------------------
 * POSE: I-POSE, NOT T-POSE
 * ---------------------------------------------------------------------------
 * `CHARACTER_COORDINATE_CONTRACT.authoredPose` says `'T'`. The runtime disagrees
 * and the runtime wins, because the clips are already written: `REST_POSE` runs
 * `LeftArm (0.185, 1.365) -> LeftForeArm (0.185, 1.095) -> LeftHand (0.185,
 * 0.845)`, i.e. the arm hangs straight down the side. The authored walk cycle
 * swings `LeftArm` by [22, 0, -4] -> [-22, 0, -4] degrees, which is a +/-22
 * degree swing about X around a *hanging* arm. Against a T-pose rest those same
 * numbers would flap a horizontal arm up and down.
 *
 * Consequence for binding: a T-posed source mesh (Tripo and most generators
 * author T) must be reposed to this I-pose before export. Bind weights in the
 * T-pose, where the limbs are well separated and automatic weighting behaves,
 * then apply the arms-down pose as the new rest.
 */

import {
  CHARACTER_BONES,
  CHARACTER_BONE_NAMES,
} from './character-asset-contract.mjs';

export const SKELETON_TARGET_VERSION = 'earthonline-humanoid-skeleton-v1';

/** Standing height of the reference adult, metres. Matches `GAIT.referenceHeight`. */
export const REFERENCE_HEIGHT = 1.75;

/**
 * Joint positions in metres, in the reference adult's rest pose, expressed in
 * WORLD space rather than as parent-local offsets.
 *
 * World space is the authoring form on purpose. Local offsets are what glTF
 * stores, but they are a poor thing to hand-edit: inserting `Spine1` between
 * `Spine` and `Spine2` changes `Spine2`'s local offset while its actual
 * position must not move a micron. Declaring absolute positions makes that
 * invariant obvious, and `localOffsetsFromWorld()` derives the glTF form.
 *
 * Joints marked (RUNTIME) are reproduced exactly from `REST_POSE` and may not
 * be changed here. The rest are new and may be tuned.
 */
export const SKELETON_WORLD_REST = Object.freeze({
  // -- root and spine ------------------------------------------------------
  Root: [0, 0, 0],
  Hips: [0, 0.92, 0], // (RUNTIME)
  Spine: [0, 1.06, 0], // (RUNTIME)
  // Spine1 is the extra break the contract asks for and the runtime does not
  // have. It sits midway so that Spine and Spine2 keep their exact pivots; no
  // existing clip targets it, so it stays identity and changes nothing.
  Spine1: [0, 1.14, 0],
  Spine2: [0, 1.22, 0], // (RUNTIME, named `Chest` in REST_POSE)
  Neck: [0, 1.42, 0], // (RUNTIME)
  Head: [0, 1.51, 0], // (RUNTIME)

  // -- face ----------------------------------------------------------------
  // Jaw pivots at the temporomandibular joint, just forward of the ear canal.
  Jaw: [0, 1.605, 0.005],
  // Eye height for a 1.75 m adult is ~0.935 of stature.
  LeftEye: [0.032, 1.635, 0.082],
  RightEye: [-0.032, 1.635, 0.082],

  // -- left arm ------------------------------------------------------------
  // The clavicle root sits near the sternum notch. It is new, so it stays
  // identity and LeftArm keeps the exact pivot REST_POSE gave it.
  LeftShoulder: [0.045, 1.365, 0],
  LeftArm: [0.185, 1.365, 0], // (RUNTIME)
  LeftForeArm: [0.185, 1.095, 0], // (RUNTIME)
  LeftHand: [0.185, 0.845, 0], // (RUNTIME)

  // Fingers hang with the arm: down in -Y, spread in X, curling slightly
  // forward in +Z. Wrist to fingertip is ~0.165 m.
  LeftHandThumb1: [0.205, 0.815, 0.022],
  LeftHandThumb2: [0.215, 0.775, 0.038],
  LeftHandThumb3: [0.221, 0.748, 0.047],
  LeftHandIndex1: [0.212, 0.755, 0.018],
  LeftHandIndex2: [0.213, 0.715, 0.020],
  LeftHandIndex3: [0.213, 0.690, 0.020],
  LeftHandMiddle1: [0.192, 0.752, 0.016],
  LeftHandMiddle2: [0.192, 0.708, 0.018],
  LeftHandMiddle3: [0.192, 0.681, 0.018],
  LeftHandRing1: [0.172, 0.755, 0.014],
  LeftHandRing2: [0.172, 0.714, 0.016],
  LeftHandRing3: [0.172, 0.689, 0.016],
  LeftHandPinky1: [0.154, 0.762, 0.012],
  LeftHandPinky2: [0.154, 0.728, 0.013],
  LeftHandPinky3: [0.154, 0.707, 0.013],

  // -- right arm (mirror of left across X) ---------------------------------
  RightShoulder: [-0.045, 1.365, 0],
  RightArm: [-0.185, 1.365, 0], // (RUNTIME)
  RightForeArm: [-0.185, 1.095, 0], // (RUNTIME)
  RightHand: [-0.185, 0.845, 0], // (RUNTIME)
  RightHandThumb1: [-0.205, 0.815, 0.022],
  RightHandThumb2: [-0.215, 0.775, 0.038],
  RightHandThumb3: [-0.221, 0.748, 0.047],
  RightHandIndex1: [-0.212, 0.755, 0.018],
  RightHandIndex2: [-0.213, 0.715, 0.020],
  RightHandIndex3: [-0.213, 0.690, 0.020],
  RightHandMiddle1: [-0.192, 0.752, 0.016],
  RightHandMiddle2: [-0.192, 0.708, 0.018],
  RightHandMiddle3: [-0.192, 0.681, 0.018],
  RightHandRing1: [-0.172, 0.755, 0.014],
  RightHandRing2: [-0.172, 0.714, 0.016],
  RightHandRing3: [-0.172, 0.689, 0.016],
  RightHandPinky1: [-0.154, 0.762, 0.012],
  RightHandPinky2: [-0.154, 0.728, 0.013],
  RightHandPinky3: [-0.154, 0.707, 0.013],

  // -- legs ----------------------------------------------------------------
  // X is +/-GAIT.stanceHalfWidth (0.084). Ankle at 0.08 is exactly
  // GAIT.soleOffset above the ground, which puts the sole at y = 0.
  LeftUpLeg: [0.084, 0.86, 0], // (RUNTIME)
  LeftLeg: [0.084, 0.48, 0], // (RUNTIME)
  LeftFoot: [0.084, 0.08, 0], // (RUNTIME)
  LeftToeBase: [0.084, 0.025, 0.115],
  RightUpLeg: [-0.084, 0.86, 0], // (RUNTIME)
  RightLeg: [-0.084, 0.48, 0], // (RUNTIME)
  RightFoot: [-0.084, 0.08, 0], // (RUNTIME)
  RightToeBase: [-0.084, 0.025, 0.115],
});

/**
 * The 17 joints the runtime actually drives today, keyed by their canonical
 * contract name and valued by the name `REST_POSE` and every authored clip
 * track uses. Only `Chest` differs; the contract renames it `Spine2` to make
 * room for a three-break spine.
 *
 * A production GLB carries canonical names. Whatever binds clips to it is
 * responsible for this mapping (`CHARACTER_IMPORT_ALIASES`).
 */
export const RUNTIME_DRIVEN_BONES = Object.freeze({
  Hips: 'Hips',
  Spine: 'Spine',
  Spine2: 'Chest',
  Neck: 'Neck',
  Head: 'Head',
  LeftArm: 'LeftArm',
  LeftForeArm: 'LeftForeArm',
  LeftHand: 'LeftHand',
  RightArm: 'RightArm',
  RightForeArm: 'RightForeArm',
  RightHand: 'RightHand',
  LeftUpLeg: 'LeftUpLeg',
  LeftLeg: 'LeftLeg',
  LeftFoot: 'LeftFoot',
  RightUpLeg: 'RightUpLeg',
  RightLeg: 'RightLeg',
  RightFoot: 'RightFoot',
});

function subtract(a, b) {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

/**
 * Convert the authored world rest into the parent-local offsets glTF stores.
 * Returns bones in `CHARACTER_BONES` order, which is the skin joint order and
 * is itself part of the contract.
 */
export function localOffsetsFromWorld(world = SKELETON_WORLD_REST) {
  return Object.freeze(CHARACTER_BONES.map(({ name, parent, category }) => {
    const here = world[name];
    if (!here) throw new Error(`skeleton target is missing a position for ${name}`);
    const origin = parent ? world[parent] : [0, 0, 0];
    if (parent && !origin) throw new Error(`skeleton target is missing parent ${parent}`);
    return Object.freeze({
      name,
      parent,
      category,
      offset: Object.freeze(subtract(here, origin)),
      world: Object.freeze([...here]),
    });
  }));
}

/** The full 56-joint skeleton, in skin joint order, with both forms present. */
export const SKELETON_TARGET = localOffsetsFromWorld();

/** Convenience lookup by bone name. */
export const SKELETON_TARGET_BY_NAME = Object.freeze(
  Object.fromEntries(SKELETON_TARGET.map((entry) => [entry.name, entry])),
);

/** Straight-line length from a joint to its parent, metres. Zero for Root. */
export function boneLength(name) {
  const entry = SKELETON_TARGET_BY_NAME[name];
  if (!entry) throw new Error(`unknown bone: ${name}`);
  const [x, y, z] = entry.offset;
  return Math.sqrt(x * x + y * y + z * z);
}

/**
 * Self-check that does not need the runtime module, so a DCC-side script can
 * call it before it exports anything.
 */
export function validateSkeletonTarget() {
  const errors = [];
  const declared = Object.keys(SKELETON_WORLD_REST);

  for (const name of CHARACTER_BONE_NAMES) {
    if (!SKELETON_WORLD_REST[name]) errors.push(`no position declared for contract bone ${name}`);
  }
  for (const name of declared) {
    if (!CHARACTER_BONE_NAMES.includes(name)) errors.push(`${name} is positioned but is not a contract bone`);
  }

  for (const entry of SKELETON_TARGET) {
    for (const axis of entry.world) {
      if (!Number.isFinite(axis)) errors.push(`${entry.name} has a non-finite position`);
    }
    if (entry.parent && boneLength(entry.name) === 0) {
      errors.push(`${entry.name} is zero-length, which most DCCs reject`);
    }
  }

  // The root pivot must sit on the ground between the feet.
  const root = SKELETON_WORLD_REST.Root;
  if (root[0] !== 0 || root[1] !== 0 || root[2] !== 0) errors.push('Root must sit at the origin');

  // Left/right must mirror exactly across X, or a mirrored walk cycle limps.
  for (const entry of SKELETON_TARGET) {
    if (!entry.name.startsWith('Left')) continue;
    const twin = SKELETON_WORLD_REST[`Right${entry.name.slice(4)}`];
    if (!twin) {
      errors.push(`${entry.name} has no Right twin`);
      continue;
    }
    const [x, y, z] = entry.world;
    if (twin[0] !== -x || twin[1] !== y || twin[2] !== z) {
      errors.push(`${entry.name} and its Right twin are not mirrored across X`);
    }
  }

  return Object.freeze({
    ok: errors.length === 0,
    version: SKELETON_TARGET_VERSION,
    boneCount: SKELETON_TARGET.length,
    referenceHeight: REFERENCE_HEIGHT,
    errors: Object.freeze(errors),
  });
}

/**
 * Serialisable form for the DCC side of the pipeline.
 *
 * The Blender scripts under `blender/` build their armature from this rather
 * than carrying their own copy of the measurements, so there is exactly one
 * place a joint can be moved and exactly one gate guarding it.
 */
export function skeletonTargetManifest() {
  return {
    version: SKELETON_TARGET_VERSION,
    contract: 'earthonline-humanoid-v1',
    referenceHeight: REFERENCE_HEIGHT,
    frame: { up: '+Y', forward: '+Z', left: '+X', handedness: 'right', metresPerUnit: 1 },
    // The rest the runtime clips are authored against. Arms hang at the sides.
    restPose: 'I',
    runtimeDrivenBones: { ...RUNTIME_DRIVEN_BONES },
    bones: SKELETON_TARGET.map(({ name, parent, category, offset, world }) => ({
      name,
      parent,
      category,
      offset: [...offset],
      world: [...world],
    })),
  };
}

// `node scripts/character-assets/skeleton-target.mjs --json` writes the manifest
// to stdout for the Blender pipeline to consume.
// The runtime (clay-player-avatar.js) imports this module in the browser,
// where there is no `process`; only run the CLI under Node.
if (typeof process !== 'undefined' && process.argv?.[1] && process.argv[1].endsWith('skeleton-target.mjs') && process.argv.includes('--json')) {
  const report = validateSkeletonTarget();
  if (!report.ok) {
    process.stderr.write(`skeleton target is invalid:\n  ${report.errors.join('\n  ')}\n`);
    process.exit(1);
  }
  process.stdout.write(`${JSON.stringify(skeletonTargetManifest(), null, 2)}\n`);
}
