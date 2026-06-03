use std::io;

/// Errors that can occur in binger-udp operations.
///
/// Covers batch overflow, buffer exhaustion, platform restrictions,
/// and I/O errors from the operating system.
///
/// # Conversions
///
/// [`std::io::Error`] is automatically converted into [`BingerError::Io`]
/// via the [`From`] trait, so `?` works naturally with I/O operations.
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

/// Convenience `Result` alias for binger-udp operations.
///
/// Equivalent to `Result<T, `[`BingerError`]`>`.
pub type BingerResult<T> = Result<T, BingerError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_full_display() {
        let err = BingerError::BatchFull { capacity: 32 };
        assert_eq!(err.to_string(), "batch is full (capacity: 32)");
    }

    #[test]
    fn buffer_too_small_display() {
        let err = BingerError::BufferTooSmall {
            required: 2048,
            available: 1024,
        };
        assert_eq!(
            err.to_string(),
            "buffer too small: need 2048 bytes, have 1024 bytes"
        );
    }

    #[test]
    fn unsupported_feature_display() {
        let err = BingerError::UnsupportedFeature { feature: "gso" };
        assert_eq!(
            err.to_string(),
            "feature `gso` is not available on this platform"
        );
    }

    #[test]
    fn io_error_from_std() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe broke");
        let err = BingerError::Io(io_err);
        assert!(err.to_string().contains("pipe broke"));
    }

    #[test]
    fn result_ok() {
        let res: BingerResult<u32> = Ok(42);
        assert_eq!(res.unwrap(), 42);
    }

    #[test]
    fn result_err() {
        let res: BingerResult<()> = Err(BingerError::BatchFull { capacity: 1 });
        assert!(res.is_err());
    }
}
