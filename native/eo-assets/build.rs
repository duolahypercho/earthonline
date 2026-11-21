// intel_tex_2 0.5.0 (the CC0 sets' BC7 and BC5 compression) links a prebuilt
// C++ static library, ispc_texcomp_astc, without the C++ runtime it needs. On
// macOS and Windows the system links that runtime anyway; on Linux the final
// link fails on `__gxx_personality_v0`. Link the GNU C++ runtime there.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}
