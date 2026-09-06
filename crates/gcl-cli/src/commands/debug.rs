//! `gcl debug`: read-only checks that run live responses through our parsers.

use anyhow::Result;
use gcl_core::Launcher;

/// Sources this command knows how to verify.
const IMPLEMENTED: &[&str] = &["mojang"];

/// Whether `verify-source` can check this source yet.
pub fn is_implemented(source: &str) -> bool {
    IMPLEMENTED.contains(&source)
}

/// Fetches Mojang's manifest and its latest release version JSON, and parses both.
///
/// Returns `false` when any endpoint failed.
pub fn verify_mojang(launcher: &Launcher) -> Result<bool> {
    let mojang = launcher.mojang();
    let manifest = match launcher.block_on(mojang.manifest()) {
        Ok(manifest) => {
            println!("PASS manifest ({} versions)", manifest.versions.len());
            manifest
        }
        Err(source) => {
            println!("FAIL manifest: {source}");
            return Ok(false);
        }
    };
    let id = manifest.latest.release.clone();
    match launcher.block_on(mojang.version(&id)) {
        Ok(version) => {
            println!("PASS version {id} ({} libraries)", version.libraries.len());
            Ok(true)
        }
        Err(source) => {
            println!("FAIL version {id}: {source}");
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mojang_is_implemented_so_far() {
        assert!(is_implemented("mojang"));
        assert!(!is_implemented("modrinth"));
    }
}
