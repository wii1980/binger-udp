// Unix-specific system abstraction.
// RawFd = c_int = i32 on all Unix platforms.

/// Unified file descriptor / socket handle type.
/// On Unix this is `RawFd` (`c_int` / `i32`).
/// On Windows this will be `SOCKET` (`usize`).
pub(crate) type Fd = std::os::fd::RawFd;

// Re-export commonly used socket-level / protocol constants.
// Linux-specific constants (UDP_GRO, UDP_SEGMENT, SCM_TIMESTAMPNS, SO_TIMESTAMPNS)
// are only available on cfg(target_os = "linux").
#[allow(unused_imports)]
pub(crate) use libc::{
    IPPROTO_IP, IPPROTO_IPV6, IPPROTO_UDP, IPV6_PKTINFO,
    IP_PKTINFO, IP_TTL, SOL_SOCKET, SO_RCVBUF, SO_SNDBUF,
};
#[cfg(target_os = "linux")]
#[allow(unused_imports)]
pub(crate) use libc::{SCM_TIMESTAMPNS, SO_TIMESTAMPNS, UDP_GRO, UDP_SEGMENT};

#[allow(dead_code)]
pub(crate) const SO_BUSY_POLL: libc::c_int = 75;

#[allow(dead_code)]
pub(crate) const SO_MAX_PACING_RATE: libc::c_int = 79;

/// Close a file descriptor / socket handle.
///
/// # Safety
///
/// The fd must be a valid, owned descriptor that has not already been closed.
/// After this call the fd is invalid and must not be used again.
#[allow(dead_code)]
pub(crate) fn close_fd(fd: Fd) {
    // SAFETY: caller guarantees fd is valid and owned.
    unsafe { libc::close(fd) };
}
