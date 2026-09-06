//! Java runtimes: finding the JVMs on this machine and installing Mojang's own.
//!
//! [`detect_all`] probes candidates on disk; [`install_runtime`] downloads a Mojang
//! runtime component into `cache/runtimes/<component>/<platform>`.

pub mod detect;
pub mod runtime;

use std::path::PathBuf;

pub use detect::{detect_all, parse_java_version, pick};
pub use runtime::{RUNTIME_MANIFEST, component_for_major, install_runtime, platform_key};

/// Errors from probing or installing a Java runtime.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A request for the runtime manifest failed.
    #[error(transparent)]
    Http(#[from] crate::http::Error),
    /// Downloading a runtime file failed.
    #[error(transparent)]
    Download(#[from] crate::download::Error),
    /// A filesystem operation on a runtime path failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The manifest has no build of this component for this platform.
    #[error("no {component} runtime for platform {platform}")]
    NoRuntimeForPlatform {
        /// The Mojang platform key.
        platform: String,
        /// The runtime component name.
        component: String,
    },
    /// Mojang publishes no runtimes for the OS and architecture this binary runs on.
    #[error("no mojang java runtime for this platform")]
    UnsupportedPlatform,
    /// A runtime manifest named a path that would escape the runtime directory.
    #[error("unsafe runtime path: {path}")]
    UnsafePath {
        /// The offending manifest path or link target.
        path: String,
    },
    /// A candidate JVM could not be run or its output could not be read.
    #[error("cannot probe java at {path}: {reason}")]
    Probe {
        /// The java binary that failed.
        path: PathBuf,
        /// Why it failed.
        reason: String,
    },
}

/// One usable Java installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaInstall {
    /// Path of the `java` binary itself.
    pub path: PathBuf,
    /// Major version: 8, 17, 21.
    pub major: u32,
    /// Full version string as the JVM reports it.
    pub version: String,
    /// Vendor string as the JVM reports it.
    pub vendor: String,
    /// How this launcher found the installation.
    pub source: JavaSource,
}

/// Where a [`JavaInstall`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaSource {
    /// Found on `PATH`.
    Path,
    /// Found through `JAVA_HOME`.
    JavaHome,
    /// Installed by this launcher from Mojang's runtime manifest.
    Mojang,
    /// Entered by the user.
    Manual,
    /// Found in a well-known system directory.
    System,
}

/// File name of the java binary on this platform.
pub const JAVA_BIN: &str = if cfg!(windows) { "java.exe" } else { "java" };
