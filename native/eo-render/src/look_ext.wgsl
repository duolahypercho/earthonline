// Group 1 (`look-bgl`) bindings 12-19: the resources of the opt-in lighting
// additions (look.rs `LightingBindings`, `LightingOptions`). Each binding
// holds a 1 x 1 stand-in until its feature binds the real resource
// (`LookGpu::set_far_shadow_map`, `set_ambient_volume`, `set_probe_grid`,
// `set_light_shadows`); a shader samples one only while its feature's bit
// is set in `look.lx[0]` (`lk_lx_on(LX_BIT_*)`, look_common.wgsl). Every
// pipeline on `look-bgl` composes this file (look.rs `LOOK_EXT_WGSL`: the
// city shader and everything built on it, the far shore, water, surf and
// prop wires), so they all compile against the same layout; unused
// declarations cost nothing.
//
// All eight are fragment-stage only. No packing: on Metal (measured on an
// M-series GPU) the largest lit pipeline layout uses 18 of 29 buffers, 24
// of 96 sampled textures and 9 of 16 samplers per stage with these in, and
// they add no sampler (the far and light shadow maps use the comparison
// sampler at binding 2, the float textures the filtering samplers at 5 or 11).

// 12: the far sun shadow map, two layers per ring (fits in look.lx[1..8], params in look.lx[9]; far_shadow.wgsl).
@group(1) @binding(12) var lx_far_shadow_map: texture_depth_2d_array;
// 13: sky visibility on a grid around the camera (origin, scale and plane mapping in look.lx[10..11]).
@group(1) @binding(13) var lx_ambient_volume: texture_3d<f32>;
// 14: sky visibility over the whole city at street level, beyond the volume.
@group(1) @binding(14) var lx_ambient_plane: texture_2d<f32>;
// 15: the probe grid's cubes, 6 layers a probe (params in look.lx[12]).
@group(1) @binding(15) var lx_probe_grid: texture_2d_array<f32>;
// 16: the probe grid's table (position, radius, state, blend weight per probe).
@group(1) @binding(16) var<storage, read> lx_probe_table: array<vec4<f32>>;
// 17: the light shadow atlas, a layer (or six) per shadowed light (params in look.lx[13]).
@group(1) @binding(17) var lx_light_shadow_atlas: texture_depth_2d_array;
// 18: the light shadow matrices, four rows a view.
@group(1) @binding(18) var<storage, read> lx_light_shadow_table: array<vec4<f32>>;
// 19 (`--shop-light`, stage N2): the shop fronts table (shop_lights.rs
// `fronts_table`; the rooms read it, shop_rooms.wgsl `sr_front`); its stand-in
// is 16's (zeros: no fronts).
@group(1) @binding(19) var<storage, read> lx_shop_fronts: array<vec4<u32>>;
