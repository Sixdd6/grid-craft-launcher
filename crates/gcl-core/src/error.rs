//! Crate-level error type, wrapping every module's error with `#[from]`.

/// Top-level error for `gcl-core`. Each module adds a variant as it lands.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O operation failed outside a module that carries its own path context.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
