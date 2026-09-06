//! Forge and NeoForge installer jars: `install_profile.json`, its data map, and its libraries.
//!
//! The installer is never executed. This module reads the profile out of the jar, resolves the
//! `data` map for side `client`, and hands the processor list to [`super::processors`].

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::Error;
use crate::download::DownloadSpec;
use crate::mojang::version::{Library, MavenCoord};
use crate::mojang::{RuleContext, rules_allow};
use crate::paths::{Root, safe_join};

/// Archive path of the jar manifest.
const MANIFEST: &str = "META-INF/MANIFEST.MF";

/// `install_profile.json` from a Forge or NeoForge installer jar.
#[derive(Debug, Deserialize)]
pub struct InstallProfile {
    /// Profile format version. Present from Forge 1.13 on.
    pub spec: Option<u32>,
    /// Human-readable profile name, e.g. `1.20.1-forge-47.4.10`.
    pub profile: Option<String>,
    /// Launcher version id the install produces.
    pub version: Option<String>,
    /// Minecraft version the build targets.
    pub minecraft: Option<String>,
    /// Archive path of the launcher version JSON, e.g. `/version.json`.
    pub json: Option<String>,
    /// Template values the processors' `{KEY}` tokens resolve through.
    #[serde(default)]
    pub data: BTreeMap<String, DataEntry>,
    /// The processors to run, in order.
    #[serde(default)]
    pub processors: Vec<Processor>,
    /// Libraries the processors need on their classpath.
    #[serde(default)]
    pub libraries: Vec<Library>,
    /// Present only on pre-1.13 profiles, which this launcher does not install.
    #[serde(default, rename = "versionInfo")]
    pub version_info: Option<serde_json::Value>,
}

impl InstallProfile {
    /// True for a pre-1.13 profile, which carries `versionInfo` and no `spec`.
    pub fn is_legacy(&self) -> bool {
        self.version_info.is_some() && self.spec.is_none()
    }
}

/// One `data` entry, which holds a separate value per side.
#[derive(Debug, Deserialize, Clone)]
pub struct DataEntry {
    /// Value used for a client install.
    pub client: String,
    /// Value used for a server install.
    pub server: String,
}

/// One installer processor: a JVM run with a jar, a classpath, and templated arguments.
#[derive(Debug, Deserialize, Clone)]
pub struct Processor {
    /// Sides this processor applies to. Empty means every side.
    #[serde(default)]
    pub sides: Vec<String>,
    /// Maven coordinate of the jar holding the processor's main class.
    pub jar: String,
    /// Extra maven coordinates for the processor's classpath.
    #[serde(default)]
    pub classpath: Vec<String>,
    /// Arguments, with `{KEY}` and `[coordinate]` tokens still in place.
    #[serde(default)]
    pub args: Vec<String>,
    /// Files the processor promises to write, as `path template` to `sha1 template`.
    #[serde(default)]
    pub outputs: BTreeMap<String, String>,
}

/// True when a processor with these `sides` takes part in a client install.
pub fn runs_on_client(sides: &[String]) -> bool {
    sides.is_empty() || sides.iter().any(|s| s == "client")
}

/// A read-only handle to an installer or processor jar.
///
/// Every method is blocking: it opens and reads a zip. Callers on the async side wrap them in
/// `tokio::task::spawn_blocking`.
pub struct InstallerJar {
    /// Path of the jar on disk.
    pub path: PathBuf,
}

impl InstallerJar {
    /// Opens a jar and checks it is a readable archive.
    pub fn open(path: PathBuf) -> Result<Self, Error> {
        let jar = InstallerJar { path };
        jar.archive()?;
        Ok(jar)
    }

    /// Reads one entry by archive name. A leading `/` is ignored.
    pub fn read_entry(&self, name: &str) -> Result<Vec<u8>, Error> {
        let mut archive = self.archive()?;
        let mut entry = archive
            .by_name(entry_name(name))
            .map_err(|source| self.zip_err(source))?;
        // Grow as the entry is read rather than trusting the size the archive declares.
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        Ok(buf)
    }

    /// Extracts every file under `prefix` into `dest`, keeping the path below the prefix.
    ///
    /// Directory entries are skipped. Returns the files written, in archive order.
    pub fn extract_prefix(&self, prefix: &str, dest: &Path) -> Result<Vec<PathBuf>, Error> {
        let mut archive = self.archive()?;
        let mut written = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|source| self.zip_err(source))?;
            let name = entry.name().replace('\\', "/");
            if entry.is_dir() || !name.starts_with(prefix) {
                continue;
            }
            let rel = &name[prefix.len()..];
            if rel.is_empty() {
                continue;
            }
            let out = safe_join(dest, rel).map_err(|_| Error::UnsafePath(name.clone()))?;
            create_parent(&out)?;
            let mut file = std::fs::File::create(&out).map_err(|source| Error::Io {
                path: out.clone(),
                source,
            })?;
            std::io::copy(&mut entry, &mut file).map_err(|source| Error::Io {
                path: out.clone(),
                source,
            })?;
            written.push(out);
        }
        Ok(written)
    }

    /// Extracts one entry into `dest_dir`, keeping its archive path, and returns where it landed.
    pub fn extract_file(&self, name: &str, dest_dir: &Path) -> Result<PathBuf, Error> {
        let rel = entry_name(name);
        let dest = safe_join(dest_dir, rel).map_err(|_| Error::UnsafePath(name.to_string()))?;
        let bytes = self.read_entry(rel)?;
        create_parent(&dest)?;
        std::fs::write(&dest, &bytes).map_err(|source| Error::Io {
            path: dest.clone(),
            source,
        })?;
        Ok(dest)
    }

    /// Reads `Main-Class` from the jar manifest, joining a wrapped continuation line.
    pub fn manifest_main_class(&self) -> Result<Option<String>, Error> {
        let bytes = match self.read_entry(MANIFEST) {
            Ok(bytes) => bytes,
            Err(Error::Zip {
                source: zip::result::ZipError::FileNotFound,
                ..
            }) => return Ok(None),
            Err(err) => return Err(err),
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut value: Option<String> = None;
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(rest) = line.strip_prefix("Main-Class:") {
                value = Some(rest.trim().to_string());
            } else if let Some(class) = value.as_mut() {
                match line.strip_prefix(' ') {
                    Some(cont) => class.push_str(cont.trim_end()),
                    None => break,
                }
            }
        }
        Ok(value.filter(|v| !v.is_empty()))
    }

    /// Opens the archive. Each call reopens the file, so no handle is held between reads.
    fn archive(&self) -> Result<zip::ZipArchive<std::fs::File>, Error> {
        let file = std::fs::File::open(&self.path).map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        zip::ZipArchive::new(file).map_err(|source| self.zip_err(source))
    }

    /// Tags a zip failure with the jar it came from.
    fn zip_err(&self, source: zip::result::ZipError) -> Error {
        Error::Zip {
            path: self.path.clone(),
            source,
        }
    }
}

/// Resolved `{KEY}` values for one side of an install.
pub struct DataMap(pub BTreeMap<String, String>);

impl DataMap {
    /// Looks one key up.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

/// Resolves the profile's `data` map for side `client` and adds the built-in keys.
///
/// Blocking: entries naming a file inside the jar are extracted into `temp`.
pub fn build_data_map(
    profile: &InstallProfile,
    jar: &InstallerJar,
    root: &Root,
    mc: &str,
    client_jar: &Path,
    temp: &Path,
) -> Result<DataMap, Error> {
    let mut map = BTreeMap::new();
    for (key, entry) in &profile.data {
        map.insert(key.clone(), resolve_value(&entry.client, jar, root, temp)?);
    }
    let builtin = [
        ("MINECRAFT_JAR", client_jar.display().to_string()),
        ("SIDE", "client".to_string()),
        ("INSTALLER", jar.path.display().to_string()),
        ("ROOT", root.path().display().to_string()),
        ("MINECRAFT_VERSION", mc.to_string()),
        ("LIBRARY_DIR", root.libraries_dir().display().to_string()),
    ];
    for (key, value) in builtin {
        map.insert(key.to_string(), value);
    }
    Ok(DataMap(map))
}

/// Substitutes `{KEY}` tokens in one argument, or resolves a bare `[coordinate]`.
pub fn substitute(arg: &str, data: &DataMap, root: &Root) -> Result<String, Error> {
    if let Some(coord) = arg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Ok(library_path(coord, root)?.display().to_string());
    }
    let mut out = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            out.push('{');
            rest = after;
            continue;
        };
        let key = &after[..end];
        let value = data
            .get(key)
            .ok_or_else(|| Error::UnknownDataKey(key.to_string()))?;
        out.push_str(value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Builds the download specs for an installer's libraries, skipping rule-excluded entries.
pub fn library_specs(
    libs: &[Library],
    root: &Root,
    rules: &RuleContext,
) -> Result<Vec<DownloadSpec>, Error> {
    let mut specs = Vec::with_capacity(libs.len());
    for lib in libs {
        if !rules_allow(&lib.rules, rules) {
            continue;
        }
        let (spec, _dest) = crate::mojang::install::library_spec(lib, root, None)?;
        specs.push(spec);
    }
    Ok(specs)
}

/// Cache path of one maven coordinate below the shared libraries directory.
pub(crate) fn library_path(name: &str, root: &Root) -> Result<PathBuf, Error> {
    let coord = MavenCoord::parse(name)?;
    Ok(safe_join(&root.libraries_dir(), &coord.path())?)
}

/// Resolves one `data` value: `[coordinate]`, `'literal'`, `/archive/path`, or plain text.
fn resolve_value(
    value: &str,
    jar: &InstallerJar,
    root: &Root,
    temp: &Path,
) -> Result<String, Error> {
    if let Some(coord) = value.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Ok(library_path(coord, root)?.display().to_string());
    }
    if let Some(literal) = value.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return Ok(literal.to_string());
    }
    if value.starts_with('/') {
        return Ok(jar.extract_file(value, temp)?.display().to_string());
    }
    Ok(value.to_string())
}

/// Strips the leading `/` a profile writes on archive paths.
fn entry_name(name: &str) -> &str {
    name.strip_prefix('/').unwrap_or(name)
}

/// Creates the parent directory of a file that is about to be written.
fn create_parent(path: &Path) -> Result<(), Error> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|source| Error::Io {
        path: parent.to_path_buf(),
        source,
    })
}

#[cfg(test)]
#[path = "forgelike_tests.rs"]
mod tests;
