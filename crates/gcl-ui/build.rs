//! Compiles `ui/app.slint` into the generated Rust module `slint::include_modules!()` pulls in.

use slint_build::{CompilerConfiguration, EmbedResourcesKind};

fn main() {
    // The `ElementHandle` API the GUI flow tests address elements with only works when the
    // generated code carries element names. A debug build emits them; a release build does
    // not, so the shipped binary stays small. `SLINT_EMIT_DEBUG_INFO=1` forces them on.
    //
    // The gate is `PROFILE`, which cargo sets to `debug` or `release`, not `DEBUG`: `DEBUG`
    // reports the debug-info level, and a release profile that turns debug info on would
    // otherwise ship the names. `cargo nextest` builds the test profile, whose `PROFILE` is
    // `debug`, so the flow tests still get them.
    println!("cargo:rerun-if-env-changed=SLINT_EMIT_DEBUG_INFO");
    println!("cargo:rerun-if-env-changed=PROFILE");
    let forced = std::env::var("SLINT_EMIT_DEBUG_INFO").is_ok_and(|value| value != "0");
    let debug_build = std::env::var("PROFILE").is_ok_and(|value| value == "debug");
    let config = CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_debug_info(forced || debug_build)
        .embed_resources(EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile ui/app.slint");
}
