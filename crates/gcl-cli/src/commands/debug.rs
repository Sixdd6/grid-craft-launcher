//! `gcl debug`: read-only checks that run live responses through our parsers.

use anyhow::Result;
use gcl_core::Launcher;
use gcl_core::instances::model::Loader;
use gcl_core::loaders::{InstallProfile, InstallerJar};
use gcl_core::mojang::VersionJson;

/// Sources this command knows how to verify.
const IMPLEMENTED: &[&str] = &["mojang", "fabric", "quilt", "forge", "neoforge"];

/// The Minecraft version most loader checks list builds for.
const CHECK_MC: &str = "1.20.1";

/// The Minecraft version the NeoForge check lists builds for.
///
/// The `net.neoforged:neoforge` artifact starts at Minecraft 1.20.2. For 1.20.1 NeoForge
/// published `net.neoforged:forge` instead, which this launcher does not read, so checking
/// [`CHECK_MC`] would report an upstream gap as a failure.
const NEOFORGE_CHECK_MC: &str = "1.20.2";

/// The Minecraft version one loader's check lists builds for.
pub fn check_mc(loader: Loader) -> &'static str {
    match loader {
        Loader::NeoForge => NEOFORGE_CHECK_MC,
        _ => CHECK_MC,
    }
}

/// Whether `verify-source` can check this source yet.
pub fn is_implemented(source: &str) -> bool {
    IMPLEMENTED.contains(&source)
}

/// The loader a source name stands for, if it names one.
pub fn loader_for(source: &str) -> Option<Loader> {
    match source {
        "fabric" => Some(Loader::Fabric),
        "quilt" => Some(Loader::Quilt),
        "forge" => Some(Loader::Forge),
        "neoforge" => Some(Loader::NeoForge),
        _ => None,
    }
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

/// Lists one loader's builds for [`check_mc`] and parses the metadata of the recommended one.
///
/// Fabric and Quilt publish a profile JSON; Forge and NeoForge publish an installer jar whose
/// `install_profile.json` this reads. Nothing is installed into an instance. Returns `false`
/// when any step failed.
pub fn verify_loader(launcher: &Launcher, loader: Loader) -> Result<bool> {
    let mc = check_mc(loader);
    let versions = match launcher.list_loader_versions(loader, mc) {
        Ok(versions) => {
            println!("PASS list {loader} {mc} ({} versions)", versions.len());
            versions
        }
        Err(source) => {
            println!("FAIL list {loader} {mc}: {source}");
            return Ok(false);
        }
    };
    let Some(picked) = versions
        .iter()
        .find(|v| v.recommended)
        .or_else(|| versions.first())
    else {
        println!("FAIL list {loader} {mc}: no builds published");
        return Ok(false);
    };
    let version = picked.version.clone();

    match loader {
        Loader::Fabric | Loader::Quilt => verify_profile(launcher, loader, mc, &version),
        Loader::Forge | Loader::NeoForge => verify_installer(launcher, loader, mc, &version),
        Loader::None => Ok(true),
    }
}

/// Fetches a Fabric-protocol profile JSON and parses it as a version JSON.
fn verify_profile(launcher: &Launcher, loader: Loader, mc: &str, version: &str) -> Result<bool> {
    let endpoints = launcher.loader_endpoints();
    let (base, api) = match loader {
        Loader::Quilt => (endpoints.quilt, gcl_core::loaders::quilt::API),
        _ => (endpoints.fabric, gcl_core::loaders::fabric::API),
    };
    let url = format!(
        "{}/{api}/versions/loader/{mc}/{version}/profile/json",
        base.trim_end_matches('/')
    );
    let http = launcher.http().clone();
    match launcher.block_on(async move { http.get_json::<VersionJson>(&url).await }) {
        Ok(profile) => {
            println!("PASS profile {}", profile.id);
            Ok(true)
        }
        Err(source) => {
            println!("FAIL profile {loader} {version}: {source}");
            Ok(false)
        }
    }
}

/// Downloads one Forge or NeoForge installer jar and parses its `install_profile.json`.
fn verify_installer(launcher: &Launcher, loader: Loader, mc: &str, version: &str) -> Result<bool> {
    let endpoints = launcher.loader_endpoints();
    let url = match loader {
        Loader::NeoForge => {
            gcl_core::loaders::neoforge::installer_url(&endpoints.neoforge, version)
        }
        _ => gcl_core::loaders::forge::installer_url(&endpoints.forge_maven, mc, version),
    };
    let dest = launcher
        .root()
        .installers_dir()
        .join(format!("verify-{loader}-{version}-installer.jar"));
    let dl = launcher.download_ctx();
    let spec = gcl_core::download::DownloadSpec {
        url,
        sha1: None,
        size: None,
        dest: dest.clone(),
        label: format!("{loader} {version} installer"),
    };
    if let Err(source) =
        launcher.block_on(async { gcl_core::download::download_one(&dl, &spec).await })
    {
        println!("FAIL installer {loader} {version}: {source}");
        return Ok(false);
    }
    match read_install_profile(&dest) {
        Ok(profile) => {
            println!(
                "PASS installer {version} ({} processors)",
                profile.processors.len()
            );
            Ok(true)
        }
        Err(source) => {
            println!("FAIL installer {loader} {version}: {source}");
            Ok(false)
        }
    }
}

/// Reads and parses `install_profile.json` out of an installer jar.
fn read_install_profile(jar: &std::path::Path) -> Result<InstallProfile> {
    let jar = InstallerJar::open(jar.to_path_buf())?;
    let bytes = jar.read_entry("install_profile.json")?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_loader_source_is_implemented() {
        assert!(is_implemented("mojang"));
        assert!(is_implemented("neoforge"));
        assert!(!is_implemented("modrinth"));
    }

    #[test]
    fn the_neoforge_check_skips_the_minecraft_version_it_never_shipped_for() {
        assert_eq!(check_mc(Loader::NeoForge), "1.20.2");
        assert_eq!(check_mc(Loader::Forge), "1.20.1");
    }

    #[test]
    fn a_loader_source_maps_onto_its_loader() {
        assert_eq!(loader_for("quilt"), Some(Loader::Quilt));
        assert_eq!(loader_for("mojang"), None);
    }
}
