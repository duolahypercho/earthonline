// (the reflect track; sky_reflect.rs `sky_light_source`) The stand-ins for
// sky_light.wgsl's `--wet-surfaces` and `--screen-reflect` sections in the
// agents' shader variant: both switches off there, whatever is compiled in.
// The agents' pipeline (agent-opaque) stands at Apple's shader compiler's
// limit (XPC_ERROR_CONNECTION_INTERRUPTED past it, the limit a compile time
// that moves with the machine's load), and every function of the module
// costs it compile time whether called or not; cars and people keep the
// wet look and the reflection they had before the switches.
const SKY_WET_SURFACES: u32 = 8388608u;
const SKY_W2_FULL: bool = false;
const SKY_W2_RING_LOBE: vec2<f32> = vec2<f32>(0.15, 0.04);
var<private> sky_w2_water: f32;
var<private> sky_w2_pud_alpha: f32;
var<private> sky_w2_ring: f32;

fn sky_w2_on() -> bool {
    return false;
}

fn sky_w2_env(e: f32) -> f32 {
    return e;
}

fn sky_w2_street(s: ptr<function, Surface>, world: vec3<f32>, n_geom: vec3<f32>, kerb: f32, asphalt: bool) {
}

fn sky_w2_generic(s: ptr<function, Surface>, ng: vec3<f32>, world: vec3<f32>, allow_puddle: bool) {
}

// (`--wet-relief`) off here too: the streak lobes as the look has them.
const SKY_WR_FULL: bool = false;
var<private> sky_wr_alpha_k: f32 = 1.0;

const SKY_SCREEN_REFLECT: u32 = 16777216u;
const SKY_SR_FULL: bool = false;
var<private> sky_sr_glass: bool;

fn sky_sr_on() -> bool {
    return false;
}

fn sky_sr_glass_mix(rad: vec3<f32>, ssr: vec4<f32>, env_i: f32) -> vec3<f32> {
    return rad;
}

fn sky_sr_ssr(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, rough: f32) -> vec4<f32> {
    return vec4<f32>(0.0);
}

fn sky_sr_spec(env: vec3<f32>, ssr: vec4<f32>, fss: vec3<f32>, so: f32) -> vec3<f32> {
    return mix(env, ssr.rgb, ssr.a) * fss * so;
}

// (`--lamp-streaks`) off here too: no rear lamps after the streak list.
fn sky_ls_lamps() -> u32 {
    return 0u;
}
