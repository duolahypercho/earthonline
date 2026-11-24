// Bake the upstream procedural forms into an indexed mesh library for Earth Online.
// The pinned donor and adapted blade geometry are MIT-licensed, see the shipped notice.
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const root = fileURLToPath(new URL('../../', import.meta.url));
const out = path.join(root, 'public/assets/ocean');
const revision = 'a596354eff400d458bc2aeb9bdf5e0a9734b2bcf';
const upstream = 'https://github.com/Noniv/abyssal-garden';
const hashes = {
  'source/src/math.js': 'a792e5547d7f8011e39231e979b8abb592e1ad306fad67ed583af5aa0e549cae',
  'source/src/world.js': '7fc2821ecc95c4284a29fab3a6d6db77998be1872a65c2831de76ccb83719e73',
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
const [math, world, license] = await Promise.all(Object.keys(hashes).map(source));
const context = vm.createContext({ LV: 0 });
vm.runInContext(math + '\n' + world, context, { timeout: 5000 });
const stem = world.indexOf('  const column = ');
const footprint = world.indexOf('    // light absorption footprint', stem);
if (stem < 0 || footprint < 0) throw Error('Missing kelp-column generator');
vm.runInContext('const big = [], glint = () => {}; const leaf = (bucket,...leaf) => bucket.push(leaf);\n' +
  world.slice(stem, footprint) + '};', context);
const run = expression => vm.runInContext(expression, context, { timeout: 5000 });

// Flatten one of the original twisted columns using a ribbed ribbon for each
// curved blade. Root coordinates are shared between blades and their stipes.
function kelp() {
  const leaves = run('rseed(140); column(0,0,0,4.0,0,0.64); big');
  const pos = [], faces = [], rgb = [];
  for (const [x,y,z,length,yaw,tilt,width,height,type,s1,s2,bend] of leaves) {
    const cy=Math.cos(yaw), sy=Math.sin(yaw), ct=Math.cos(tilt), st=Math.sin(tilt);
    const first=pos.length/3, rows=type === 1 ? 4 : 7;
    for (let j=0;j<rows;j++) {
      const t=j/(rows-1), a=(bend-0.30)*3.4, curve=Math.abs(a)<0.06?0.06:a;
      const forward=Math.sin(curve*t)/curve, arch=(1-Math.cos(curve*t))/curve;
      const sinShape=Math.max(0,Math.sin(Math.PI*t**0.75));
      const profile = type===1 ? 0.42 : 0.02+0.98*sinShape**0.72;
      for (const side of [-1,0,1]) {
        const edge=side*width*profile;
        const ruffle=type===1 ? 0 : (Math.sin(10*t+s2*30)*0.26*side*side+0.20*(1-Math.abs(side)))*width*profile;
        pos.push((x+length*(st*cy*forward+ct*cy*arch)-sy*edge+ct*cy*ruffle)/4,
          (y+length*(ct*forward-st*arch)-st*ruffle)/4,
          (z+length*(st*sy*forward+ct*sy*arch)+cy*edge+ct*sy*ruffle)/4);
        const pigment=(0.65+0.28*t+0.18*s1)*(side===0?1.14:0.93);
        rgb.push(...[0.21+0.10*s2,0.17+0.05*s1,0.038].map(c=>c*pigment));
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
  const mesh=recipe?run(recipe):kelp(), p=Float32Array.from(mesh.p), indices=Uint16Array.from(mesh.i);
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
    colors[4*i+3]=255;
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
  adaptation:'Generated rock, sponge, fan and fish geometry. The original kelp-column generator supplies blade positions, sizes and bends. Vertex pigments and game-scale placement are adapted to the Pacific coast.'
},null,2)+'\n');
console.log(gltf.meshes.map(x=>x.name).join(', '),glb.length,sha(glb));
