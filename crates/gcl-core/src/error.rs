//! Crate-level error type, wrapping every module's error with `#[from]`.

/// Top-level error for `gcl-core`. Each module adds a variant as it lands.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O operation failed outside a module that carries its own path context.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// An error resolving or creating the app root layout.
    #[error(transparent)]
    Paths(#[from] crate::paths::Error),
    /// An error loading or saving the config file.
    #[error(transparent)]
    Config(#[from] crate::config::Error),
    /// An HTTP request or streamed download failed.
    #[error(transparent)]
    Http(#[from] crate::http::Error),
    /// A download or cache operation failed.
    #[error(transparent)]
    Download(#[from] crate::download::Error),
}
