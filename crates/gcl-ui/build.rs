//! Compiles `ui/app.slint` into the generated Rust module `slint::include_modules!()` pulls in.

use slint_build::{CompilerConfiguration, EmbedResourcesKind};

fn main() {
    // The `ElementHandle` API the GUI flow tests address elements with only works when the
    // generated code carries element names. A debug build emits them; a release build does
    // not, so the shipped binary stays small. `SLINT_EMIT_DEBUG_INFO=1` forces them on.
    println!("cargo:rerun-if-env-changed=SLINT_EMIT_DEBUG_INFO");
    let forced = std::env::var("SLINT_EMIT_DEBUG_INFO").is_ok_and(|value| value != "0");
    let debug_build = std::env::var("DEBUG").is_ok_and(|value| value != "false" && value != "0");
    let config = CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_debug_info(forced || debug_build)
        .embed_resources(EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile ui/app.slint");
}
