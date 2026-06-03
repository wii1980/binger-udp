pub(crate) use imp::*;

#[cfg(all(target_os = "linux", not(feature = "miri-safe")))]
pub(crate) mod linux;
#[cfg(all(target_os = "linux", not(feature = "miri-safe")))]
pub(crate) use linux as imp;

#[cfg(all(target_os = "macos", not(feature = "miri-safe")))]
pub(crate) mod macos;
#[cfg(all(target_os = "macos", not(feature = "miri-safe")))]
pub(crate) use macos as imp;

#[cfg(all(target_os = "windows", not(feature = "miri-safe")))]
pub(crate) mod windows;
#[cfg(all(target_os = "windows", not(feature = "miri-safe")))]
pub(crate) use windows as imp;

#[cfg(any(
    feature = "miri-safe",
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
pub(crate) mod fallback;
#[cfg(any(
    feature = "miri-safe",
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
pub(crate) use fallback as imp;
