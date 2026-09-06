//! Finds the JVMs installed on this machine and reads their version and vendor.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Error, JAVA_BIN, JavaInstall, JavaSource};
use crate::paths::Root;

/// How long a single `java -XshowSettings:properties -version` probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Probes every candidate JVM and returns the ones that answered.
#[tracing::instrument(skip(root))]
pub async fn detect_all(root: &Root) -> Vec<JavaInstall> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (path, source) in candidates(root) {
        let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen.insert(key) {
            continue;
        }
        match probe(&path, source).await {
            Ok(install) => out.push(install),
            Err(err) => tracing::debug!(%err, "skipping a java candidate"),
        }
    }
    out
}

/// Every java binary worth probing, in preference order, before deduplication.
fn candidates(root: &Root) -> Vec<(PathBuf, JavaSource)> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        out.push((
            PathBuf::from(home).join("bin").join(JAVA_BIN),
            JavaSource::JavaHome,
        ));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(JAVA_BIN);
            if candidate.is_file() {
                out.push((candidate, JavaSource::Path));
            }
        }
    }
    for dir in [
        "/usr/lib/jvm",
        "/usr/lib64/jvm",
        "/Library/Java/JavaVirtualMachines",
    ] {
        out.extend(scan_homes(Path::new(dir), JavaSource::System));
    }
    if let Some(home) = std::env::var_os("HOME") {
        out.extend(scan_homes(
            &PathBuf::from(home).join(".jdks"),
            JavaSource::System,
        ));
    }
    out.extend(scan_runtimes(&root.runtimes_dir()));
    out.retain(|(p, _)| p.is_file());
    out
}

/// Looks for `<dir>/*/bin/java` and the macOS `<dir>/*/Contents/Home/bin/java` layout.
fn scan_homes(dir: &Path, source: JavaSource) -> Vec<(PathBuf, JavaSource)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let home = entry.path();
        out.push((home.join("bin").join(JAVA_BIN), source));
        out.push((
            home.join("Contents")
                .join("Home")
                .join("bin")
                .join(JAVA_BIN),
            source,
        ));
    }
    out
}

/// Walks `cache/runtimes` for any `bin/java` this launcher installed.
fn scan_runtimes(dir: &Path) -> Vec<(PathBuf, JavaSource)> {
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out
}

/// Depth-limited walk collecting `bin/java` binaries.
fn walk(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, JavaSource)>) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let candidate = path.join("bin").join(JAVA_BIN);
            if candidate.is_file() {
                out.push((candidate, JavaSource::Mojang));
            }
            walk(&path, depth + 1, out);
        }
    }
}

/// Runs one JVM's property dump and turns it into a [`JavaInstall`].
pub async fn probe(path: &Path, source: JavaSource) -> Result<JavaInstall, Error> {
    let fail = |reason: String| Error::Probe {
        path: path.to_path_buf(),
        reason,
    };
    let run = tokio::process::Command::new(path)
        .arg("-XshowSettings:properties")
        .arg("-version")
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(PROBE_TIMEOUT, run)
        .await
        .map_err(|_| fail("timed out".to_string()))?
        .map_err(|err| fail(err.to_string()))?;

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let version = property(&text, "java.version")
        .ok_or_else(|| fail("no java.version property in the output".to_string()))?;
    let (major, version) = parse_java_version(&version)
        .ok_or_else(|| fail(format!("cannot read a major version from {version:?}")))?;
    Ok(JavaInstall {
        path: path.to_path_buf(),
        major,
        version,
        vendor: property(&text, "java.vendor").unwrap_or_default(),
        source,
    })
}

/// Reads one `name = value` line out of `-XshowSettings:properties` output.
fn property(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(name)?;
        let value = rest.trim_start().strip_prefix('=')?;
        Some(value.trim().to_string())
    })
}

/// Reads the major version out of a Java version string, keeping the string itself.
pub fn parse_java_version(s: &str) -> Option<(u32, String)> {
    let trimmed = s.trim();
    let head: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = head.split('.').filter(|p| !p.is_empty());
    let first: u32 = parts.next()?.parse().ok()?;
    let major = if first == 1 {
        parts.next()?.parse().ok()?
    } else {
        first
    };
    Some((major, trimmed.to_string()))
}

/// Picks a JVM for a wanted major version: the exact major, else the lowest above it.
pub fn pick(installs: &[JavaInstall], major: u32) -> Option<&JavaInstall> {
    installs.iter().find(|i| i.major == major).or_else(|| {
        installs
            .iter()
            .filter(|i| i.major > major)
            .min_by_key(|i| i.major)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(major: u32) -> JavaInstall {
        JavaInstall {
            path: PathBuf::from(format!("/jvm/{major}/bin/java")),
            major,
            version: major.to_string(),
            vendor: "Test".to_string(),
            source: JavaSource::Manual,
        }
    }

    #[test]
    fn parses_the_legacy_and_modern_version_schemes() {
        assert_eq!(parse_java_version("1.8.0_392").map(|(m, _)| m), Some(8));
        assert_eq!(parse_java_version("17.0.9").map(|(m, _)| m), Some(17));
        assert_eq!(parse_java_version("25.0.4.1").map(|(m, _)| m), Some(25));
        assert_eq!(parse_java_version("21").map(|(m, _)| m), Some(21));
        assert_eq!(parse_java_version("22-ea").map(|(m, _)| m), Some(22));
        assert_eq!(
            parse_java_version("17.0.9").map(|(_, v)| v),
            Some("17.0.9".to_string())
        );
    }

    #[test]
    fn rejects_a_version_string_it_cannot_read() {
        assert!(parse_java_version("").is_none());
        assert!(parse_java_version("banana").is_none());
        assert!(parse_java_version("1").is_none());
    }

    #[test]
    fn pick_prefers_the_exact_major() {
        let all = [install(8), install(17), install(21)];
        assert_eq!(pick(&all, 17).map(|i| i.major), Some(17));
    }

    #[test]
    fn pick_falls_back_to_the_lowest_higher_major() {
        let all = [install(8), install(21), install(17)];
        assert_eq!(pick(&all, 16).map(|i| i.major), Some(17));
        assert_eq!(pick(&all, 22), None);
    }

    #[test]
    fn pick_on_an_empty_list_is_none() {
        assert!(pick(&[], 17).is_none());
    }

    #[test]
    fn reads_properties_out_of_the_settings_dump() {
        let dump =
            "Property settings:\n    java.vendor = Eclipse Adoptium\n    java.version = 17.0.9\n";
        assert_eq!(property(dump, "java.version"), Some("17.0.9".to_string()));
        assert_eq!(
            property(dump, "java.vendor"),
            Some("Eclipse Adoptium".to_string())
        );
        assert_eq!(property(dump, "java.home"), None);
    }

    #[tokio::test]
    async fn detect_all_returns_without_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let found = detect_all(&root).await;
        let on_path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join(JAVA_BIN).is_file()))
            .unwrap_or(false);
        if on_path {
            assert!(!found.is_empty(), "java is on PATH but none was detected");
            for install in &found {
                assert!(install.major >= 1, "{install:?}");
            }
        }
    }

    #[tokio::test]
    async fn probing_a_missing_binary_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("no-such-java");
        assert!(matches!(
            probe(&missing, JavaSource::Manual).await,
            Err(Error::Probe { .. })
        ));
    }
}
