// Unix implementation of socket address encoding/decoding and raw syscall wrappers.
use crate::sys::Fd;
use std::mem;
use std::net::SocketAddr;

pub(crate) fn encode_sockaddr(
    addr: SocketAddr,
    storage: &mut libc::sockaddr_storage,
) -> libc::socklen_t {
    storage.ss_family = 0;
    match addr {
        SocketAddr::V4(v4) => {
            #[allow(clippy::unnecessary_cast)]
            let raw: libc::sockaddr_in = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: v4.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(v4.ip().octets()),
                },
                #[cfg(target_os = "macos")]
                sin_len: 0,
                sin_zero: [0; 8],
            };
            // SAFETY: storage is a valid mutable reference to sockaddr_storage,
            // which is large enough to hold sockaddr_in.
            unsafe {
                let dst = storage as *mut _ as *mut libc::sockaddr_in;
                dst.write(raw);
            }
            mem::size_of::<libc::sockaddr_in>() as libc::socklen_t
        }
        SocketAddr::V6(v6) => {
            #[allow(clippy::unnecessary_cast)]
            let raw: libc::sockaddr_in6 = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: v6.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                },
                sin6_scope_id: v6.scope_id(),
                #[cfg(target_os = "macos")]
                sin6_len: 0,
            };
            // SAFETY: storage is a valid mutable reference to sockaddr_storage,
            // which is large enough to hold sockaddr_in6.
            unsafe {
                let dst = storage as *mut _ as *mut libc::sockaddr_in6;
                dst.write(raw);
            }
            mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t
        }
    }
}

pub(crate) fn decode_sockaddr(
    storage: &libc::sockaddr_storage,
    len: libc::socklen_t,
) -> SocketAddr {
    if len == 0 {
        return SocketAddr::V4(std::net::SocketAddrV4::new(
            std::net::Ipv4Addr::UNSPECIFIED,
            0,
        ));
    }
    match storage.ss_family as i32 {
        libc::AF_INET => {
            // SAFETY: storage contains a valid sockaddr_in when ss_family == AF_INET
            let sin: &libc::sockaddr_in = unsafe { &*(storage as *const _ as *const _) };
            let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            let port = u16::from_be(sin.sin_port);
            SocketAddr::V4(std::net::SocketAddrV4::new(ip, port))
        }
        libc::AF_INET6 => {
            // SAFETY: storage contains a valid sockaddr_in6 when ss_family == AF_INET6
            let sin6: &libc::sockaddr_in6 = unsafe { &*(storage as *const _ as *const _) };
            let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
            let port = u16::from_be(sin6.sin6_port);
            SocketAddr::V6(std::net::SocketAddrV6::new(
                ip,
                port,
                sin6.sin6_flowinfo,
                sin6.sin6_scope_id,
            ))
        }
        _ => SocketAddr::V4(std::net::SocketAddrV4::new(
            std::net::Ipv4Addr::UNSPECIFIED,
            0,
        )),
    }
}

pub(crate) fn raw_sendto(fd: Fd, data: &[u8], addr: SocketAddr) -> std::io::Result<usize> {
    // SAFETY: zeroed() produces valid initialization for sockaddr_storage
    let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
    let addr_len = encode_sockaddr(addr, &mut storage);
    // SAFETY: sendto with valid fd, data pointer, and sockaddr
    let ret = unsafe {
        libc::sendto(
            fd,
            data.as_ptr().cast(),
            data.len(),
            0,
            &storage as *const _ as *const libc::sockaddr,
            addr_len,
        )
    };
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub(crate) fn raw_send(fd: Fd, data: &[u8]) -> std::io::Result<usize> {
    // SAFETY: send with valid fd and data pointer, for connected sockets
    let ret = unsafe { libc::send(fd, data.as_ptr().cast(), data.len(), 0) };
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub(crate) fn raw_recvfrom(fd: Fd, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
    // SAFETY: zeroed() produces valid initialization for sockaddr_storage
    let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
    let mut addr_len = mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    // SAFETY: recvfrom with valid fd, buf pointer, and sockaddr output
    let ret = unsafe {
        libc::recvfrom(
            fd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            0,
            &mut storage as *mut _ as *mut libc::sockaddr,
            &mut addr_len,
        )
    };
    if ret >= 0 {
        Ok((ret as usize, decode_sockaddr(&storage, addr_len)))
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub(crate) fn raw_getsockname(fd: Fd) -> std::io::Result<SocketAddr> {
    // SAFETY: zeroed() produces valid initialization for sockaddr_storage
    let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
    let mut len = mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    // SAFETY: getsockname with valid fd and sockaddr output
    let ret =
        unsafe { libc::getsockname(fd, &mut storage as *mut _ as *mut libc::sockaddr, &mut len) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(decode_sockaddr(&storage, len))
}

pub(crate) fn raw_connect(fd: Fd, addr: SocketAddr) -> std::io::Result<()> {
    // SAFETY: zeroed() produces valid initialization for sockaddr_storage
    let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
    let addr_len = encode_sockaddr(addr, &mut storage);
    // SAFETY: connect with valid fd and sockaddr
    let ret = unsafe { libc::connect(fd, &storage as *const _ as *const libc::sockaddr, addr_len) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(crate) fn raw_setsockopt(
    fd: Fd,
    level: libc::c_int,
    optname: libc::c_int,
    val: libc::c_int,
) -> std::io::Result<()> {
    // SAFETY: setsockopt with valid fd, level, optname, and value pointer
    let ret = unsafe {
        libc::setsockopt(
            fd,
            level,
            optname,
            &val as *const _ as *const libc::c_void,
            mem::size_of_val(&val) as libc::socklen_t,
        )
    };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
pub(crate) fn raw_setsockopt_u32(
    fd: Fd,
    level: libc::c_int,
    optname: libc::c_int,
    val: u32,
) -> std::io::Result<()> {
    // SAFETY: setsockopt with valid fd, level, optname, and u32 value pointer
    let ret = unsafe {
        libc::setsockopt(
            fd,
            level,
            optname,
            &val as *const _ as *const libc::c_void,
            mem::size_of_val(&val) as libc::socklen_t,
        )
    };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
pub(crate) fn raw_setsockopt_timeval(
    fd: Fd,
    level: libc::c_int,
    optname: libc::c_int,
    usecs: u32,
) -> std::io::Result<()> {
    // SAFETY: timeval is zero-initialized then fields set
    let tv = libc::timeval {
        tv_sec: (usecs / 1_000_000) as libc::time_t,
        tv_usec: (usecs % 1_000_000) as libc::suseconds_t,
    };
    // SAFETY: setsockopt with valid fd, level, optname, and timeval pointer
    let ret = unsafe {
        libc::setsockopt(
            fd,
            level,
            optname,
            &tv as *const _ as *const libc::c_void,
            mem::size_of_val(&tv) as libc::socklen_t,
        )
    };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(crate) fn raw_getsockopt(
    fd: Fd,
    level: libc::c_int,
    optname: libc::c_int,
) -> std::io::Result<libc::c_int> {
    let mut val: libc::c_int = 0;
    let mut len = mem::size_of_val(&val) as libc::socklen_t;
    // SAFETY: getsockopt with valid fd, level, optname, and output pointers
    let ret = unsafe {
        libc::getsockopt(
            fd,
            level,
            optname,
            &mut val as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(val)
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::net::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn encode_decode_v4_loopback() {
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let len = encode_sockaddr(addr, &mut storage);
        let decoded = decode_sockaddr(&storage, len);
        assert_eq!(addr, decoded);
    }

    #[test]
    fn encode_decode_v4_broadcast() {
        let addr: SocketAddr = "255.255.255.255:0".parse().unwrap();
        let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let len = encode_sockaddr(addr, &mut storage);
        let decoded = decode_sockaddr(&storage, len);
        assert_eq!(addr, decoded);
    }

    #[test]
    fn encode_decode_v4_unspecified() {
        let addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let len = encode_sockaddr(addr, &mut storage);
        let decoded = decode_sockaddr(&storage, len);
        assert_eq!(addr, decoded);
    }

    #[test]
    fn encode_decode_v6_loopback() {
        let addr: SocketAddr = "[::1]:8080".parse().unwrap();
        let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let len = encode_sockaddr(addr, &mut storage);
        let decoded = decode_sockaddr(&storage, len);
        assert_eq!(addr, decoded);
    }

    #[test]
    fn encode_decode_v6_unspecified() {
        let addr: SocketAddr = "[::]:0".parse().unwrap();
        let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let len = encode_sockaddr(addr, &mut storage);
        let decoded = decode_sockaddr(&storage, len);
        assert_eq!(addr, decoded);
    }

    #[test]
    fn encode_decode_v6_with_flowinfo_scope_id() {
        let addr = SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
            12345,
            0xabcdef,
            42,
        ));
        let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let len = encode_sockaddr(addr, &mut storage);
        let decoded = decode_sockaddr(&storage, len);
        assert_eq!(addr, decoded);
    }

    #[test]
    fn decode_zero_len_returns_v4_unspecified() {
        let storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let decoded = decode_sockaddr(&storage, 0);
        assert_eq!(
            decoded,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        );
    }

    #[test]
    fn decode_unknown_family_returns_v4_unspecified() {
        let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        storage.ss_family = 0xFF;
        let len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        let decoded = decode_sockaddr(&storage, len);
        assert_eq!(
            decoded,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        );
    }

    #[test]
    fn raw_sendto_recvfrom_roundtrip() {
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        let recv_addr = recv.local_addr().unwrap();
        let send = UdpSocket::bind("127.0.0.1:0").unwrap();
        let send_addr = send.local_addr().unwrap();

        let data = b"hello udp";
        let n = raw_sendto(send.as_raw_fd(), data, recv_addr).unwrap();
        assert_eq!(n, data.len());

        let mut buf = [0u8; 64];
        let (n, src) = raw_recvfrom(recv.as_raw_fd(), &mut buf).unwrap();
        assert_eq!(n, data.len());
        assert_eq!(&buf[..n], data);
        assert_eq!(src, send_addr);
    }

    #[test]
    fn raw_sendto_recvfrom_empty_payload() {
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        let recv_addr = recv.local_addr().unwrap();
        let send = UdpSocket::bind("127.0.0.1:0").unwrap();
        let send_addr = send.local_addr().unwrap();

        let n = raw_sendto(send.as_raw_fd(), b"", recv_addr).unwrap();
        assert_eq!(n, 0);

        let mut buf = [0u8; 64];
        let (n, src) = raw_recvfrom(recv.as_raw_fd(), &mut buf).unwrap();
        assert_eq!(n, 0);
        assert_eq!(src, send_addr);
    }

    #[test]
    fn raw_send_connected_recvfrom() {
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        let recv_addr = recv.local_addr().unwrap();
        let send = UdpSocket::bind("127.0.0.1:0").unwrap();
        let send_addr = send.local_addr().unwrap();

        raw_connect(send.as_raw_fd(), recv_addr).unwrap();

        let data = b"connected data";
        let n = raw_send(send.as_raw_fd(), data).unwrap();
        assert_eq!(n, data.len());

        let mut buf = [0u8; 64];
        let (n, src) = raw_recvfrom(recv.as_raw_fd(), &mut buf).unwrap();
        assert_eq!(n, data.len());
        assert_eq!(&buf[..n], data);
        assert_eq!(src, send_addr);
    }

    #[test]
    fn raw_getsockname_returns_bound_addr() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = raw_getsockname(sock.as_raw_fd()).unwrap();
        assert_eq!(addr.ip(), "127.0.0.1".parse::<IpAddr>().unwrap());
        assert!(addr.port() > 0, "should have a non-zero OS-assigned port");
    }

    #[test]
    fn raw_connect_to_peer() {
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        let recv_addr = recv.local_addr().unwrap();
        let send = UdpSocket::bind("127.0.0.1:0").unwrap();

        raw_connect(send.as_raw_fd(), recv_addr).unwrap();

        let data = b"connect test";
        raw_send(send.as_raw_fd(), data).unwrap();

        let mut buf = [0u8; 64];
        let (n, _) = raw_recvfrom(recv.as_raw_fd(), &mut buf).unwrap();
        assert_eq!(&buf[..n], data);
    }

    #[test]
    fn raw_setsockopt_getsockopt_ttl_roundtrip() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let fd = sock.as_raw_fd();

        raw_setsockopt(fd, libc::IPPROTO_IP, libc::IP_TTL, 128).unwrap();
        let val = raw_getsockopt(fd, libc::IPPROTO_IP, libc::IP_TTL).unwrap();
        assert_eq!(val, 128);
    }

    #[test]
    fn raw_setsockopt_getsockopt_rcvbuf_roundtrip() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let fd = sock.as_raw_fd();

        raw_setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF, 65536).unwrap();
        let val = raw_getsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF).unwrap();
        assert!(val >= 65536, "SO_RCVBUF should be >= 65536, got {val}");
    }

    #[test]
    fn raw_setsockopt_u32_works() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let fd = sock.as_raw_fd();

        raw_setsockopt_u32(fd, libc::IPPROTO_IP, libc::IP_TTL, 64u32).unwrap();
        let val = raw_getsockopt(fd, libc::IPPROTO_IP, libc::IP_TTL).unwrap();
        assert_eq!(val, 64);
    }

    #[test]
    fn raw_setsockopt_timeval_rcvtimeo() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let fd = sock.as_raw_fd();

        raw_setsockopt_timeval(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, 100_000).unwrap();

        let mut buf = [0u8; 64];
        let err = raw_recvfrom(fd, &mut buf).unwrap_err();
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::WouldBlock,
            "recvfrom on empty socket with SO_RCVTIMEO should return WouldBlock"
        );
    }
}
