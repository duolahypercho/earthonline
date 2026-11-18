/**
 * EarthOnline authored humanoid contract.
 *
 * Tripo, Blender, or another DCC may be used before this boundary.
 * Runtime assets cross the boundary only after their skeleton has been
 * normalized to this exact vocabulary and hierarchy. The contract is shared
 * by the local player, hero NPCs, ordinary near-tier NPCs, garments, props,
 * and the animation library.
 */

export const CHARACTER_ASSET_CONTRACT_VERSION = 'earthonline-humanoid-v1';

function bone(name, parent, category = 'body') {
  return Object.freeze({ name, parent, category });
}

function hand(side) {
  const handName = `${side}Hand`;
  const entries = [
    bone(`${side}Shoulder`, 'Spine2', 'arm'),
    bone(`${side}Arm`, `${side}Shoulder`, 'arm'),
    bone(`${side}ForeArm`, `${side}Arm`, 'arm'),
    bone(handName, `${side}ForeArm`, 'hand'),
  ];
  for (const finger of ['Thumb', 'Index', 'Middle', 'Ring', 'Pinky']) {
    entries.push(
      bone(`${handName}${finger}1`, handName, 'finger'),
      bone(`${handName}${finger}2`, `${handName}${finger}1`, 'finger'),
      bone(`${handName}${finger}3`, `${handName}${finger}2`, 'finger'),
    );
  }
  return entries;
}

function leg(side) {
  return [
    bone(`${side}UpLeg`, 'Hips', 'leg'),
    bone(`${side}Leg`, `${side}UpLeg`, 'leg'),
    bone(`${side}Foot`, `${side}Leg`, 'foot'),
    bone(`${side}ToeBase`, `${side}Foot`, 'foot'),
  ];
}

/**
 * Exact joint order used by skin.joints. It deliberately follows common
 * game-rig bone semantics without vendor prefixes and adds an explicit root plus
 * jaw/eye joints required by a playable hero. Facial identity and expressions
 * still belong in morph targets; this is not a facial-bone rig.
 */
export const CHARACTER_BONES = Object.freeze([
  bone('Root', null, 'root'),
  bone('Hips', 'Root', 'body'),
  bone('Spine', 'Hips', 'body'),
  bone('Spine1', 'Spine', 'body'),
  bone('Spine2', 'Spine1', 'body'),
  bone('Neck', 'Spine2', 'body'),
  bone('Head', 'Neck', 'head'),
  bone('Jaw', 'Head', 'face'),
  bone('LeftEye', 'Head', 'face'),
  bone('RightEye', 'Head', 'face'),
  ...hand('Left'),
  ...hand('Right'),
  ...leg('Left'),
  ...leg('Right'),
]);

export const CHARACTER_BONE_NAMES = Object.freeze(CHARACTER_BONES.map(({ name }) => name));

/** Bone aliases are import-only. Production GLBs must contain canonical names. */
export const CHARACTER_IMPORT_ALIASES = Object.freeze({
  Chest: 'Spine2',
  LeftUpperArm: 'LeftArm',
  LeftForearm: 'LeftForeArm',
  RightUpperArm: 'RightArm',
  RightForearm: 'RightForeArm',
  LeftUpperLeg: 'LeftUpLeg',
  LeftShin: 'LeftLeg',
  RightUpperLeg: 'RightUpLeg',
  RightShin: 'RightLeg',
});

export const CHARACTER_COORDINATE_CONTRACT = Object.freeze({
  units: 'metres',
  metresPerUnit: 1,
  up: '+Y',
  forward: '+Z',
  handedness: 'right',
  authoredPose: 'T',
  rootPivot: 'ground-center',
  rootMotion: 'Root-only',
});

/**
 * Clips are stored once and applied to every conforming body and garment.
 * Locomotion is in-place because simulation owns world movement.
 */
export const CHARACTER_ANIMATION_CONTRACT = Object.freeze({
  library: 'earthonline-humanoid-motion-v1',
  requiredClips: Object.freeze([
    'idle',
    'walk',
    'run',
    'sprint',
    'sit',
    'drive',
    'enter-vehicle',
    'exit-vehicle',
    'combat-idle',
    'aim',
    'melee',
    'hit-react',
    'downed',
    'get-up',
  ]),
  inPlaceClips: Object.freeze(['idle', 'walk', 'run', 'sprint', 'combat-idle', 'aim', 'melee']),
  trackTargets: 'canonical-bone-names',
});

export const CHARACTER_ASSET_LIMITS = Object.freeze({
  sourceBody: Object.freeze({ triangleMin: 8_000, triangleMax: 12_000 }),
  npcLod0: Object.freeze({ triangleMax: 12_000, materialMax: 4, morphTargetMax: 16 }),
  playerLod0: Object.freeze({ triangleMax: 24_000, materialMax: 6, morphTargetMax: 48 }),
  garmentLod0: Object.freeze({ triangleMax: 6_000, materialMax: 2, morphTargetMax: 0 }),
  influencesPerVertexMax: 4,
  textureEdgeMax: 2_048,
});

export const CHARACTER_RECIPE_CONTRACT = Object.freeze({
  version: 'earthonline-character-recipe-v1',
  fixedCatalogOnly: true,
  slots: Object.freeze(['body', 'head', 'hair', 'top', 'bottom', 'shoes', 'accessory']),
  bodyShapeParameters: Object.freeze(['height', 'mass', 'muscle', 'age']),
  faceShapeParameters: Object.freeze(['head', 'jaw', 'nose', 'eyes', 'mouth']),
  animationLibrary: CHARACTER_ANIMATION_CONTRACT.library,
});

export function validateCharacterContract() {
  const errors = [];
  const seen = new Set();
  for (const entry of CHARACTER_BONES) {
    if (!entry.name || seen.has(entry.name)) errors.push(`duplicate or empty bone: ${entry.name}`);
    if (entry.parent != null && !seen.has(entry.parent)) {
      errors.push(`${entry.name} parent ${entry.parent} must appear first`);
    }
    seen.add(entry.name);
  }
  if (CHARACTER_BONES[0]?.name !== 'Root' || CHARACTER_BONES[0]?.parent !== null) {
    errors.push('Root must be the first and only parentless bone');
  }
  if (CHARACTER_BONES.filter(({ parent }) => parent === null).length !== 1) {
    errors.push('the skeleton must contain exactly one parentless bone');
  }
  return Object.freeze({
    ok: errors.length === 0,
    version: CHARACTER_ASSET_CONTRACT_VERSION,
    boneCount: CHARACTER_BONES.length,
    errors: Object.freeze(errors),
  });
}
