use std::io;

/// Errors that can occur in binger-udp operations.
///
/// Covers batch overflow and I/O errors from the operating system.
///
/// # Conversions
///
/// [`std::io::Error`] is automatically converted into [`BingerError::Io`]
/// via the [`From`] trait, so `?` works naturally with I/O operations.
#[derive(Debug, thiserror::Error)]
#[allow(clippy::module_name_repetitions)]
pub enum BingerError {
    /// Batch is full — no more entries can be added.
    #[error("batch is full (capacity: {capacity})")]
    BatchFull { capacity: usize },

    /// Transparent stdio error from the OS.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}
