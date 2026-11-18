#!/usr/bin/env node
// Round-trip metric check for the exported SF patrol GLB.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import './node-glb-image-shim.mjs';
import * as THREE from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const file = path.resolve(process.argv[2] || path.join(HERE, 'out/sf-patrol-sedan-v1.glb'));
const bytes = fs.readFileSync(file);
const gltf = await new GLTFLoader().parseAsync(
  bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), '',
);
const scene = gltf.scene;
scene.updateMatrixWorld(true);

const box = (object) => new THREE.Box3().setFromObject(object, true);
const round = (value) => Number(value.toFixed(6));
const wholeBox = box(scene);
const overall = wholeBox.getSize(new THREE.Vector3());
const body = scene.getObjectByName('Body');
const bodySize = body ? box(body).getSize(new THREE.Vector3()) : new THREE.Vector3();
const wheelNames = ['Wheel_FL', 'Wheel_FR', 'Wheel_RL', 'Wheel_RR'];
const wheelData = wheelNames.map((name) => {
  const wheel = scene.getObjectByName(name);
  if (!wheel) return { name, present: false };
  const wheelBounds = box(wheel);
  const size = wheelBounds.getSize(new THREE.Vector3());
  const center = wheel.getWorldPosition(new THREE.Vector3());
  const diameter = Math.max(size.y, size.z);
  return {
    name,
    present: true,
    centerM: center.toArray().map(round),
    diameterM: round(diameter),
    groundErrorM: round(Math.abs(center.y - diameter / 2)),
  };
});
const frontZ = wheelData.filter((wheel) => wheel.name.includes('_F')).map((wheel) => wheel.centerM?.[2]);
const rearZ = wheelData.filter((wheel) => wheel.name.includes('_R')).map((wheel) => wheel.centerM?.[2]);
const wheelbase = frontZ.length === 2 && rearZ.length === 2
  ? Math.abs((frontZ[0] + frontZ[1]) / 2 - (rearZ[0] + rearZ[1]) / 2)
  : null;
const occupantRoot = scene.getObjectByName('Occupant_FL');
const driver = scene.getObjectByName('Officer_Driver_Seated');
const driverBounds = driver ? box(driver) : new THREE.Box3();
const windscreen = scene.getObjectByName('Windscreen');
const windscreenMaterial = Array.isArray(windscreen?.material)
  ? windscreen.material[0] : windscreen?.material;
const lightbarNames = [];
scene.traverse((object) => {
  if (object.name.startsWith('Lightbar_')) lightbarNames.push(object.name);
});

const checks = {
  overallLength: Math.abs(overall.z - 4.95) <= 0.01,
  overallHeight: Math.abs(overall.y - 1.52) <= 0.01,
  bodyWidth: Math.abs(bodySize.x - 1.90) <= 0.015,
  allFourWheelNodes: wheelData.length === 4 && wheelData.every((wheel) => wheel.present),
  wheelbase: wheelbase !== null && Math.abs(wheelbase - 2.90) <= 0.005,
  tyreDiameters: wheelData.every((wheel) => wheel.present && Math.abs(wheel.diameterM - 0.70) <= 0.005),
  tyreGrounding: wheelData.every((wheel) => wheel.present && wheel.groundErrorM <= 0.01),
  seatedDriver: !!occupantRoot && !!driver && driver.parent === occupantRoot,
  driverInsideCabin: !!driver && driverBounds.min.y >= 0.28 && driverBounds.max.y <= 1.35,
  transparentWindscreen: !!windscreenMaterial && windscreenMaterial.transparent
    && windscreenMaterial.opacity >= 0.1 && windscreenMaterial.opacity <= 0.45,
  blueRedLightbar: lightbarNames.filter((name) => name.includes('_Blue_') && name.endsWith('_Lens')).length === 4
    && lightbarNames.filter((name) => name.includes('_Red_') && name.endsWith('_Lens')).length === 4,
};
const report = {
  file: path.relative(path.resolve(HERE, '../../..'), file),
  units: 'metres',
  gltfBoundsM: {
    length: round(overall.z),
    overallWidthIncludingAccessories: round(overall.x),
    height: round(overall.y),
    bodyWidthIncludingSurfaceDecals: round(bodySize.x),
  },
  wheelbaseM: wheelbase === null ? null : round(wheelbase),
  wheels: wheelData,
  driverBoundsM: driver ? {
    min: driverBounds.min.toArray().map(round),
    max: driverBounds.max.toArray().map(round),
  } : null,
  windscreenOpacity: windscreenMaterial?.opacity ?? null,
  lightbarParts: lightbarNames.length,
  checks,
  passed: Object.values(checks).every(Boolean),
};
console.log(JSON.stringify(report, null, 2));
process.exit(report.passed ? 0 : 1);
