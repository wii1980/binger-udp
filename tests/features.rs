#[cfg(all(target_os = "linux", feature = "gso"))]
#[path = "features/gso.rs"]
mod gso;

#[cfg(all(target_os = "linux", feature = "gro"))]
#[path = "features/gro.rs"]
mod gro;

#[cfg(all(target_os = "linux", feature = "timestamping"))]
#[path = "features/timestamping.rs"]
mod timestamping;

#[cfg(all(target_os = "linux", feature = "pktinfo"))]
#[path = "features/pktinfo.rs"]
mod pktinfo;

#[cfg(feature = "metrics")]
#[path = "features/metrics_test.rs"]
mod metrics_test;

#[cfg(all(target_os = "linux", feature = "pacing"))]
#[path = "features/pacing.rs"]
mod pacing;

#[cfg(all(target_os = "linux", feature = "busy-poll"))]
#[path = "features/busy_poll.rs"]
mod busy_poll;
