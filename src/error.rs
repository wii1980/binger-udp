use std::io;

/// Errors that can occur in binger operations.
#[derive(Debug, thiserror::Error)]
pub enum BingerError {
    /// Batch is full — no more entries can be added.
    #[error("batch is full (capacity: {capacity})")]
    BatchFull { capacity: usize },

    /// Buffer too small for received datagram.
    #[error("buffer too small: need {required} bytes, have {available} bytes")]
    BufferTooSmall { required: usize, available: usize },

    /// Platform does not support the requested feature.
    #[error("feature `{feature}` is not available on this platform")]
    UnsupportedFeature { feature: &'static str },

    /// Transparent stdio error from the OS.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

/// Convenience `Result` alias.
pub type BingerResult<T> = Result<T, BingerError>;
