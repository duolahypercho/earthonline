# wgpu-hal 30.0.2, modified for Earth Online

This folder is a copy of the `wgpu-hal` crate, version 30.0.2, from the wgpu
project (<https://github.com/gfx-rs/wgpu>), published on crates.io. The
workspace uses it in place of the crates.io release through
`[patch.crates-io]` in `native/Cargo.toml`.

- Source: crates.io `wgpu-hal-30.0.2.crate`, SHA-256
  `a57452fe453658e141aafdb09de1028afa498805fab518238ddf372a518813ff`
  (the checksum `native/Cargo.lock` recorded for the release before the patch).
- Licence: MIT OR Apache-2.0, as upstream. The texts are in
  [LICENSE.MIT](LICENSE.MIT) and [LICENSE.APACHE](LICENSE.APACHE).
- Copyright: the wgpu developers.

## What was changed

Only the Metal backend's shader compile options, so that one shader module can
be compiled in Metal's relaxed math mode instead of Apple's default, fast math:

- `src/metal/mod.rs`: `ShaderModule` has a `relaxed_math` field, and the
  constant `RELAXED_MATH_LABEL_MARK` (`"[relaxed-math]"`) is added.
- `src/metal/device.rs`: `create_shader_module` sets `relaxed_math` when the
  module's label contains that mark; `load_shader` then sets
  `MTLMathMode::Relaxed` with `MTLMathFloatingPointFunctions::Fast` for that
  module (macOS 15, iOS 18, tvOS 18, visionOS 2 and later; before them, where
  the math mode does not exist, `setFastMathEnabled(false)`). Every other
  module is compiled exactly as upstream compiles it.

Each changed place carries an "Earth Online change" comment. Not functional:
`src/vulkan/descriptor.rs` was converted from CRLF to LF line endings (the
repository's text files are LF), and the registry's `.cargo-ok` marker file was
not copied.

## Why

With fast math, Apple's shader compiler miscompiles the agents' draw shader
(`eo-render` `agents_draw.wgsl` with `look_light.wgsl`) when the cinematic
look, `--grime`, `--ambient-volume`, lamp or headlight shadows, `--far-shadows`
and `--cascade-blend` are all compiled in: the sun colour the car paint's
clearcoat reads comes out with too little red, and the clearcoat's sharp lobe
turns it into saturated cyan speckles (about 500 pixels a frame on the
kit-street camera). Changes to the source that leave its arithmetic alone move
or remove the speckles, and with IEEE arithmetic no NaN or infinity occurs on
the shader's lighting path, so the fault lies in the compiler's fast-math code
generation rather than in a line of the shader.

Measured on an Apple M-series Mac (macOS 27.2), same camera and switches, the
mode set for that one module only: fast math 570 cyan pixels; relaxed, safe,
fast math off, and fast math with precise functions all 0. Frame time at the
`high` tier against fast math (kit_street, street_walk, drive; two interleaved
rounds on a busy machine): relaxed -0.13 to -0.08 ms, fast math with precise
functions +0.25 to +0.52 ms, safe +0.38 to +0.73 ms, fast math off +0.83 to
+1.52 ms. Relaxed is the fast mode without its assumption that there is no NaN
or infinity, at no measurable cost. Turning fast math off for every shader is
not an option: Apple's compiler then fails on the far city's draw shader
(`hlod-draw`). wgpu has no option for the math mode, hence the patch.

The renderer marks the agents' draw module only when the cinematic look is
compiled in (`eo-render/src/agents.rs`, `draw_module_label`); without it the
module compiles as before. `eo-app --no-agents-relaxed-math` drops the mark.

## Updating wgpu

On a wgpu upgrade, copy the new `wgpu-hal` release here, apply the same change
(search the old copy for "Earth Online change"), record the new crate's
SHA-256 above, and run `cargo test --release -p eo-render --test
agents_relaxed_math`. Drop this folder and the `[patch.crates-io]` entry if
wgpu gains its own option for the Metal math mode.
