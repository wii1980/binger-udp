// Platform-specific socket address encoding/decoding and raw syscall wrappers.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;

#[cfg(windows)]
mod windows_impl;
#[cfg(windows)]
pub(crate) use windows_impl::*;
