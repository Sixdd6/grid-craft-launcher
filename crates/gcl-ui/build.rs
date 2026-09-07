//! Compiles `ui/app.slint` into the generated Rust module `slint::include_modules!()` pulls in.

use slint_build::{CompilerConfiguration, EmbedResourcesKind};

fn main() {
    let config = CompilerConfiguration::new()
        .with_style("fluent".into())
        .embed_resources(EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile ui/app.slint");
}
