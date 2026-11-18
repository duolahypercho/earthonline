// GLTFLoader requires browser image globals while inspecting a textured GLB
// in Node. Geometry and material checks only need nonempty embedded images.
globalThis.self = globalThis;
globalThis.createImageBitmap = async (blob) => {
  if (!blob.size) throw new Error('GLB contains an empty texture image');
  return { width: 1, height: 1 };
};
