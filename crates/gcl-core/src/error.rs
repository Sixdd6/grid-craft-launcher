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
    /// An instance could not be created, read, or deleted.
    #[error(transparent)]
    Instance(#[from] crate::instances::Error),
    /// An HTTP request or streamed download failed.
    #[error(transparent)]
    Http(#[from] crate::http::Error),
    /// A download or cache operation failed.
    #[error(transparent)]
    Download(#[from] crate::download::Error),
    /// Java detection or a Mojang runtime install failed.
    #[error(transparent)]
    Java(#[from] crate::java::Error),
    /// A Mojang metadata fetch, cache read, or parse failed.
    #[error(transparent)]
    Mojang(#[from] crate::mojang::Error),
    /// A mod loader could not be listed or installed.
    #[error(transparent)]
    Loaders(#[from] crate::loaders::Error),
}
