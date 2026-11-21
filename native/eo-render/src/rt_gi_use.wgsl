// (`--rt-gi`, `--rt-reflect`; rt_composite.rs) The city shader's side of the
// ray-traced passes: a thin G-buffer the city's opaque pass writes beside its
// colour, read by the traces (the normal, the specular lobe) and the deferred
// composite (rt_composite.wgsl), which replaces the rig's indirect diffuse
// and its environment reflection with the traced ones in the same frame. The
// colour is the frame without the options: the hook in look_light.wgsl
// (rt_composite.rs `HOOK`) only records what the composite needs. Composed
// into the city shaders only with an option (look.rs
// `city_shader_source_variant_probes`); without them this code, the hook and
// the entry point are absent and every pipeline compiles as before.
//
// The values (rt_composite.rs `GSlot`; `fs_rtg`, generated after this, writes
// those the options use):
//   a: rgb the indirect diffuse added (as it reached the colour: after the
//      fog and any coat over it), a the environment's intensity class;
//   b: rgb what multiplies an irradiance into that colour, a the screen-space
//      visibility used;
//   n: rgb the vertex normal x 0.5 + 0.5, a 1 (shaded);
//   c: rgb the environment reflection added (the sky or probe along the
//      reflection, or the screen-space reflection, as it reached the colour),
//      a the surface's roughness;
//   d: rgb what multiplies a reflected radiance into that colour (Fresnel x
//      the specular occlusion, the fog and coats), a the environment's class.

var<private> rtg_a: vec4<f32>;
var<private> rtg_b: vec4<f32>;
var<private> rtg_n: vec4<f32>;
var<private> rtg_c: vec4<f32>;
var<private> rtg_d: vec4<f32>;
