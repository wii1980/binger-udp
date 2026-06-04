#[cfg(all(target_os = "linux", feature = "gso", not(feature = "miri-safe")))]
#[path = "features/gso.rs"]
mod gso;

#[cfg(all(target_os = "linux", feature = "gro", not(feature = "miri-safe")))]
#[path = "features/gro.rs"]
mod gro;

#[cfg(all(
    target_os = "linux",
    feature = "timestamping",
    not(feature = "miri-safe")
))]
#[path = "features/timestamping.rs"]
mod timestamping;

#[cfg(all(target_os = "linux", feature = "pktinfo", not(feature = "miri-safe")))]
#[path = "features/pktinfo.rs"]
mod pktinfo;

#[cfg(feature = "metrics")]
#[path = "features/metrics_test.rs"]
mod metrics_test;

#[cfg(all(target_os = "linux", feature = "pacing", not(feature = "miri-safe")))]
#[path = "features/pacing.rs"]
mod pacing;

#[cfg(all(target_os = "linux", feature = "busy-poll", not(feature = "miri-safe")))]
#[path = "features/busy_poll.rs"]
mod busy_poll;
