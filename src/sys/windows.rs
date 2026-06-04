//! Windows-specific system abstraction.
//! SOCKET = usize on 64-bit Windows.

/// Windows socket handle type (SOCKET = usize).
pub(crate) type Fd = usize;

// Socket-level / protocol constants, cast to i32 for compatibility
// with the Unix-side code in socket.rs that passes them to setsockopt.
pub(crate) const SOL_SOCKET: i32 = windows_sys::Win32::Networking::WinSock::SOL_SOCKET as i32;
pub(crate) const SO_SNDBUF: i32 = windows_sys::Win32::Networking::WinSock::SO_SNDBUF as i32;
pub(crate) const SO_RCVBUF: i32 = windows_sys::Win32::Networking::WinSock::SO_RCVBUF as i32;
pub(crate) const IPPROTO_IP: i32 = windows_sys::Win32::Networking::WinSock::IPPROTO_IP as i32;
pub(crate) const IP_TTL: i32 = windows_sys::Win32::Networking::WinSock::IP_TTL as i32;
#[allow(dead_code)]
pub(crate) const AF_INET: i32 = windows_sys::Win32::Networking::WinSock::AF_INET as i32;
#[allow(dead_code)]
pub(crate) const AF_INET6: i32 = windows_sys::Win32::Networking::WinSock::AF_INET6 as i32;

/// Close a socket handle.
///
/// # Safety
///
/// The fd must be a valid, owned socket handle that has not already
/// been closed.  After this call the fd is invalid and must not be
/// used again.
#[allow(dead_code)]
pub(crate) fn close_fd(fd: Fd) {
    // SAFETY: caller guarantees fd is valid and owned.
    unsafe {
        windows_sys::Win32::Networking::WinSock::closesocket(fd);
    }
}
