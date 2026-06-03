// Unix-specific system abstraction.
// RawFd = c_int = i32 on all Unix platforms.

/// Unified file descriptor / socket handle type.
/// On Unix this is `RawFd` (`c_int` / `i32`).
/// On Windows this will be `SOCKET` (`usize`).
pub(crate) type Fd = std::os::fd::RawFd;

/// Invalid fd sentinel value.
#[allow(dead_code)]
pub(crate) const INVALID_FD: Fd = -1;

// Re-export commonly used socket-level / protocol constants.
#[allow(unused_imports)]
pub(crate) use libc::{
    AF_INET, AF_INET6, IPPROTO_IP, IPPROTO_IPV6, IPPROTO_UDP, IP_PKTINFO, IP_TTL,
    IPV6_PKTINFO, IPV6_RECVPKTINFO, SCM_TIMESTAMPNS, SOL_SOCKET, SO_RCVBUF, SO_SNDBUF,
    SO_TIMESTAMPNS, UDP_GRO, UDP_SEGMENT,
};

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
