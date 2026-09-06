//! Running Forge and NeoForge installer processors as child JVM processes.
//!
//! Task 3 adds the implementation; this file declares the seam the loaders call through.

use std::path::{Path, PathBuf};

/// Runs one installer processor and reports the exit code it finished with.
#[async_trait::async_trait]
pub trait ProcessRunner: Send + Sync {
    /// Runs `java -cp <classpath> <main> <args>`, writing the process output to `log`.
    async fn run(
        &self,
        java: &Path,
        classpath: &[PathBuf],
        main: &str,
        args: &[String],
        log: &Path,
    ) -> Result<i32, std::io::Error>;
}
