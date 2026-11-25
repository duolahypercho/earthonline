// Bake the upstream procedural forms into an indexed mesh library for Earth Online.
// The pinned donor and adapted blade geometry are MIT-licensed, see the shipped notice.
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { coastalKelp } from './coastal-kelp.mjs';
import { bullKelpBed, eelgrassBed, streamingCrown, openCrown, clusteredCrown } from './coastal-plants.mjs';

import { bedrockLedge, fracturedBoulder } from './reef-rocks.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const out = path.join(root, 'public/assets/ocean');
const revision = 'a596354eff400d458bc2aeb9bdf5e0a9734b2bcf';
const upstream = 'https://github.com/Noniv/abyssal-garden';
const hashes = {
  'source/src/math.js': 'a792e5547d7f8011e39231e979b8abb592e1ad306fad67ed583af5aa0e549cae',
  'source/src/world.js': '7fc2821ecc95c4284a29fab3a6d6db77998be1872a65c2831de76ccb83719e73',
  'source/src/shaders.js': 'a4744f7b16a2c3930e5097175937e6556fd3d82b668fbcb457154981eec87b29',
  LICENSE: 'b108a76735ab7cc0eab686fc3e85ca668bdb8866977093846d01bac5b6440453',
};
const sha = data => createHash('sha256').update(data).digest('hex');
async function source(name) {
  let bytes;
  if (process.argv[2]) {
    bytes = await readFile(path.join(process.argv[2], path.basename(name)));
  } else {
    const response = await fetch('https://raw.githubusercontent.com/Noniv/abyssal-garden/' + revision + '/' + name);
    if (!response.ok) throw Error(name + ' returned ' + response.status);
    bytes = Buffer.from(await response.arrayBuffer());
  }
  if (sha(bytes) !== hashes[name]) throw Error('Source digest mismatch for ' + name);
  return bytes.toString('utf8');
}
const [math, world, shaders, license] = await Promise.all(Object.keys(hashes).map(source));
if (!shaders.includes('const W_LEAF =')) throw Error('Missing source leaf shader');
const stems = [];
const context = vm.createContext({
  LV: 0,
  sampleStipes(stem, strands, height) {
    stems.length = 0;
    for (let strand = 0; strand < strands; strand++) {
      stems.push(Array.from({length: 17}, (_, k) => {
        const y = height * k / 16;
        const [x, z] = stem(strand, y);
        return [x, y, z];
      }));
    }
  },
});
vm.runInContext(math + '\n' + world, context, { timeout: 5000 });
const stem = world.indexOf('  const column = ');
const footprint = world.indexOf('    // light absorption footprint', stem);
if (stem < 0 || footprint < 0) throw Error('Missing kelp-column generator');
vm.runInContext('const big = [], glint = () => {}; const leaf = (bucket,...leaf) => bucket.push(leaf);\n' +
  world.slice(stem, footprint).replace('    for (let s = 0; s < strands;', '    sampleStipes(stem, strands, H);\n    for (let s = 0; s < strands;') + '};', context);
const run = expression => vm.runInContext(expression, context, { timeout: 5000 });

// Flatten one of the original twisted columns using a ribbed ribbon for each
// curved blade. Root coordinates are shared between blades and their stipes.
function kelp() {
  const leaves = run('rseed(140); column(0,0,0,4.0,0,0.64); big');
  const pos = [], faces = [], rgb = [];
  for (const [x,y,z,length,yaw,tilt,width,height,type,s1,s2,bend] of leaves) {
    const cy=Math.cos(yaw), sy=Math.sin(yaw), ct=Math.cos(tilt), st=Math.sin(tilt);
    const first=pos.length/3, rows=type === 1 ? 4 : 7;
    // The source leaf shader's close geometry at t=0. The game's vertex shader
    // supplies the continuing current. A middle lane gives the blade its midrib.
    const stipe = type === 1;
    const wave = stipe ? 0.3 : 1.9;
    const flex = stipe ? 0.3 : 1;
    const k = ((bend-0.3)*2 + Math.sin(s2*40+height*0.25)*0.1*flex)*1.7;
    const curve = Math.abs(k)<0.06 ? 0.06 : k;
    const pigment = stipe ? [0.10,0.11,0.035] : s2>0.72 ? [0.25,0.17,0.025]
      : [0.11-0.07*s1,0.125-0.025*s1,0.022+0.018*s1];
    for (let j=0;j<rows;j++) {
      const v = Math.sin(j/(rows-1)*Math.PI*0.5);
      const phase = v*(3+5*s2)+s1*30;
      const wobble = Math.sin(phase)*0.12*wave;
      const swing = Math.sin(curve*v), lift = Math.cos(curve*v);
      const arch = (1-lift)/curve+wobble*v;
      const axial = swing/curve;
      const slope = swing+wobble+Math.cos(phase)*0.10*wave*v*(3+5*s2);
      const normLength = Math.hypot(lift,slope);
      const nx = (ct*lift-st*slope)/normLength, ny=(-st*lift-ct*slope)/normLength;
      const twist = (s1-0.5)*2.4*v + Math.sin(s2*20)*0.25*v;
      const shape = Math.max(0,Math.sin(Math.PI*v**(stipe?0.05:0.5)))**(stipe?0.1:0.32);
      for (const side of [-1,0,1]) {
        const edge=side*width*0.5*shape;
        const ruffle=Math.sin(v*10+s2*30)*width*0.36*side*Math.sin(Math.PI*v);
        const rib=stipe?0:0.04*width*shape*(1-Math.abs(side));
        const across=edge*Math.cos(twist)-(ruffle+rib)*Math.sin(twist);
        const normal=edge*Math.sin(twist)+(ruffle+rib)*Math.cos(twist);
        const radial=length*(st*axial+ct*arch)+nx*normal;
        pos.push((x+cy*radial-sy*across)/4,
          (y+length*(ct*axial-st*arch)+ny*normal)/4,
          (z+sy*radial+cy*across)/4);
        const shade=(0.80+0.20*v)*(side===0 ? 1.15 : 0.96);
        rgb.push(...pigment.map(c=>c*shade));
      }
    }
    for (let j=0;j<rows-1;j++) for(let k=0;k<2;k++) {
      const i=first+3*j+k;faces.push(i,i+3,i+1,i+1,i+3,i+4);
    }
  }
  return {p:pos,i:faces,rgb};
}

const recipes = [
  ['reef-block', 'meshChunk(1,2,true)', [.175,.18,.145]],
  ['reef-outcrop', 'meshChunk(7,2,true)', [.23,.188,.138]],
  ['cup-sponge', 'meshLathe([[0,0,.5],[.5,0,.7],[.72,.18,1],[.74,.38,1],[.62,.5,.9],[.48,.42,.45],[.4,.2,.15],[0,.14,.06]],18,.25,10)', [.54,.235,.09]],
  ['tube-sponge', 'meshTubes()', [.49,.29,.16]],
  ['branching-colony', 'meshCoral(1,false)', [.49,.175,.109]],
  ['kelp-column', null, null],
  ['baitfish', 'meshFish()', [.36,.44,.43]],
  ['coastal-kelp', null, null],
  ['bull-kelp-crown', bullKelpBed, null],
  ['eelgrass-shoots', eelgrassBed, null],
  ['streaming-crown', streamingCrown, null],
  ['open-crown', openCrown, null],
  ['clustered-crown', clusteredCrown, null],
  ['bedrock-ledge', bedrockLedge, null],
  ['fractured-boulder', fracturedBoulder, null],
];
const gltf={asset:{version:'2.0',generator:'Earth Online reef cook'},scene:0,scenes:[{nodes:[]}],nodes:[],meshes:[],bufferViews:[],accessors:[],buffers:[]};
const chunks=[];let length=0;
function accessor(array,type,components,componentType,target,bounds=false,normalized=false) {
  const padding=(4-length%4)%4;chunks.push(Buffer.alloc(padding));length+=padding;
  const buf=Buffer.from(array.buffer,array.byteOffset,array.byteLength);
  const view=gltf.bufferViews.push({buffer:0,byteOffset:length,byteLength:buf.length,target})-1;
  chunks.push(buf);length+=buf.length;
  const a={bufferView:view,componentType,type,count:array.length/components};
  if(normalized)a.normalized=true;
  if(bounds){a.min=Array(components).fill(Infinity);a.max=Array(components).fill(-Infinity);
    array.forEach((v,i)=>{const k=i%components;a.min[k]=Math.min(a.min[k],v);a.max[k]=Math.max(a.max[k],v);});}
  return gltf.accessors.push(a)-1;
}
for(const [name,recipe,tint] of recipes){
  let mesh;
  if (typeof recipe === 'function') mesh = recipe();
  else if (name === 'coastal-kelp') mesh = coastalKelp(run('big'), stems);
  else mesh = recipe ? run(recipe) : kelp();
  const p=Float32Array.from(mesh.p), indices=Uint16Array.from(mesh.i);
  if (p.length/3>65535 || mesh.i.some(i=>i<0 || i>=p.length/3)) throw Error('Index overflow '+name);
  const normals=new Float32Array(p.length), colors=new Uint8Array(p.length/3*4);
  for(let i=0;i<indices.length;i+=3){
    const a=indices[i]*3, b=indices[i+1]*3, c=indices[i+2]*3;
    const x=p[b]-p[a],y=p[b+1]-p[a+1],z=p[b+2]-p[a+2],u=p[c]-p[a],v=p[c+1]-p[a+1],w=p[c+2]-p[a+2];
    for(const j of [a,b,c]){normals[j]+=y*w-z*v;normals[j+1]+=z*u-x*w;normals[j+2]+=x*v-y*u;}
  }
  for(let i=0;i<p.length/3;i++){
    const k=i*3, n=Math.hypot(...normals.subarray(k,k+3))||1;
    const pigment=mesh.aux ? 0.15+0.85*mesh.aux[i] : 1;
    for(let j=0;j<3;j++){
      normals[k+j]/=n;
      colors[4*i+j]=Math.round(255*Math.min(1,mesh.rgb?mesh.rgb[k+j]:tint[j]*pigment));
    }
    colors[4*i+3]=mesh.bladeCoord ? mesh.bladeCoord[i] : 255;
  }
  const attributes={POSITION:accessor(p,'VEC3',3,5126,34962,true),NORMAL:accessor(normals,'VEC3',3,5126,34962),COLOR_0:accessor(colors,'VEC4',4,5121,34962,false,true)};
  const m=gltf.meshes.push({name,primitives:[{attributes,indices:accessor(indices,'SCALAR',1,5123,34963),mode:4}]})-1;
  gltf.scenes[0].nodes.push(gltf.nodes.push({name,mesh:m})-1);
}
chunks.push(Buffer.alloc((4-length%4)%4));const binary=Buffer.concat(chunks);gltf.buffers=[{byteLength:binary.length}];
let header=Buffer.from(JSON.stringify(gltf));header=Buffer.concat([header,Buffer.alloc((4-header.length%4)%4,32)]);
const frame=Buffer.alloc(20), binHeader=Buffer.alloc(8);
frame.write('glTF');frame.writeUInt32LE(2,4);frame.writeUInt32LE(28+header.length+binary.length,8);
frame.writeUInt32LE(header.length,12);frame.write('JSON',16);binHeader.writeUInt32LE(binary.length);binHeader.write('BIN\0',4);
const glb=Buffer.concat([frame,header,binHeader,binary]);
await writeFile(path.join(out,'reef-forms.glb'),glb);
await writeFile(path.join(out,'reef-forms.LICENSE.txt'),license);
await writeFile(path.join(out,'reef-forms.provenance.json'),JSON.stringify({
  url:'/assets/ocean/reef-forms.glb',bytes:glb.length,sha256:sha(glb),author:'Maksymilian Dendura and Earth Online contributors',
  source:upstream,revision,inputs:hashes,license:'MIT',licenseFile:'reef-forms.LICENSE.txt',builder:'scripts/blender-ocean/cook-reef.mjs',
  originalPlantMeshes: {
    'bull-kelp-crown': {author:'Earth Online contributors',reference:'https://farallones.noaa.gov/media/docs/201904-bull-kelp-recovery-plan.pdf'},
    'eelgrass-shoots': {author:'Earth Online contributors',reference:'https://plants.sc.egov.usda.gov/DocumentLibrary/factsheet/pdf/fs_zoma.pdf'},
  },
  originalReefMeshes: {
    kelp: ['streaming-crown','open-crown','clustered-crown'],
    rock: ['bedrock-ledge','fractured-boulder'],
    author:'Earth Online contributors',
    geologyReference:'https://pubs.usgs.gov/ds/781/OffshoreSanFrancisco/metadata/Habitat_OffshoreSanFrancisco_metadata_faq.html',
  },
  adaptation:'Generated rock, sponge, fan and fish geometry. The source kelp-column generator and leaf shader supply blade placement, taper, width, twist, pigment and edge ruffling. The coastal variant joins blades and gas bladders to continuous sampled stipes, with per-vertex blade flex. Vertex pigments and game-scale placement are adapted to the Pacific coast.'
},null,2)+'\n');
console.log(gltf.meshes.map(x=>x.name).join(', '),glb.length,sha(glb));
