// Windows implementation of socket address encoding/decoding and raw syscall wrappers.
use std::io;
use std::mem;
use std::net::SocketAddr;
use windows_sys::Win32::Networking::WinSock as WS;

use crate::sys::Fd;

pub(crate) fn encode_sockaddr(addr: SocketAddr, storage: &mut WS::SOCKADDR_STORAGE) -> i32 {
    match addr {
        SocketAddr::V4(v4) => {
            let sin = WS::SOCKADDR_IN {
                sin_family: WS::AF_INET,
                sin_port: v4.port().to_be(),
                sin_addr: WS::IN_ADDR {
                    S_un: WS::IN_ADDR_0 {
                        S_addr: u32::from_ne_bytes(v4.ip().octets()),
                    },
                },
                sin_zero: [0i8; 8],
            };
            // SAFETY: storage is large enough to hold SOCKADDR_IN
            unsafe {
                *(storage as *mut _ as *mut WS::SOCKADDR_IN) = sin;
            }
            mem::size_of::<WS::SOCKADDR_IN>() as i32
        }
        SocketAddr::V6(v6) => {
            // SAFETY: IN6_ADDR_0 is a union; we only initialise the Byte field.
            // The Word field remains uninitialised, which is fine for a union.
            let sin6 = WS::SOCKADDR_IN6 {
                sin6_family: WS::AF_INET6,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: v6.flowinfo(),
                sin6_addr: WS::IN6_ADDR {
                    u: WS::IN6_ADDR_0 {
                        Byte: v6.ip().octets(),
                    },
                },
                Anonymous: WS::SOCKADDR_IN6_0 {
                    sin6_scope_id: v6.scope_id(),
                },
            };
            // SAFETY: storage is large enough to hold SOCKADDR_IN6
            unsafe {
                *(storage as *mut _ as *mut WS::SOCKADDR_IN6) = sin6;
            }
            mem::size_of::<WS::SOCKADDR_IN6>() as i32
        }
    }
}

pub(crate) fn decode_sockaddr(storage: &WS::SOCKADDR_STORAGE, len: i32) -> SocketAddr {
    if len == 0 {
        return SocketAddr::V4(std::net::SocketAddrV4::new(
            std::net::Ipv4Addr::UNSPECIFIED,
            0,
        ));
    }
    match storage.ss_family as i32 {
        x if x == WS::AF_INET as i32 => {
            // SAFETY: storage contains a valid SOCKADDR_IN when ss_family == AF_INET
            let sin: &WS::SOCKADDR_IN =
                unsafe { &*(storage as *const _ as *const WS::SOCKADDR_IN) };
            // SAFETY: sin_addr.S_un.S_addr is a union, accessing the u32 field
            let ip = std::net::Ipv4Addr::from(u32::from_be(unsafe { sin.sin_addr.S_un.S_addr }));
            let port = u16::from_be(sin.sin_port);
            SocketAddr::V4(std::net::SocketAddrV4::new(ip, port))
        }
        x if x == WS::AF_INET6 as i32 => {
            // SAFETY: storage contains a valid SOCKADDR_IN6 when ss_family == AF_INET6
            let sin6: &WS::SOCKADDR_IN6 =
                unsafe { &*(storage as *const _ as *const WS::SOCKADDR_IN6) };
            // SAFETY: union field access for sin6_addr.u.Byte and Anonymous
            let ip = std::net::Ipv6Addr::from(unsafe { sin6.sin6_addr.u.Byte });
            let port = u16::from_be(sin6.sin6_port);
            SocketAddr::V6(std::net::SocketAddrV6::new(
                ip,
                port,
                sin6.sin6_flowinfo,
                unsafe { sin6.Anonymous.sin6_scope_id },
            ))
        }
        _ => SocketAddr::V4(std::net::SocketAddrV4::new(
            std::net::Ipv4Addr::UNSPECIFIED,
            0,
        )),
    }
}

pub(crate) fn raw_getsockname(fd: Fd) -> io::Result<SocketAddr> {
    // SAFETY: zeroed() produces valid initialization for SOCKADDR_STORAGE
    let mut storage: WS::SOCKADDR_STORAGE = unsafe { mem::zeroed() };
    let mut len = mem::size_of::<WS::SOCKADDR_STORAGE>() as i32;
    // SAFETY: getsockname with valid fd and sockaddr output
    let ret = unsafe { WS::getsockname(fd, &mut storage as *mut _ as *mut WS::SOCKADDR, &mut len) };
    if ret == 0 {
        Ok(decode_sockaddr(&storage, len))
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn raw_connect(fd: Fd, addr: SocketAddr) -> io::Result<()> {
    // SAFETY: zeroed() produces valid initialization for SOCKADDR_STORAGE
    let mut storage: WS::SOCKADDR_STORAGE = unsafe { mem::zeroed() };
    let addr_len = encode_sockaddr(addr, &mut storage);
    // SAFETY: connect with valid fd and sockaddr
    let ret = unsafe { WS::connect(fd, &storage as *const _ as *const WS::SOCKADDR, addr_len) };
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn raw_setsockopt(fd: Fd, level: i32, optname: i32, val: i32) -> io::Result<()> {
    // SAFETY: setsockopt with valid fd, level, optname, and value pointer
    let ret = unsafe {
        WS::setsockopt(
            fd,
            level,
            optname,
            &val as *const _ as *const u8,
            mem::size_of_val(&val) as i32,
        )
    };
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn raw_sendto(fd: Fd, data: &[u8], addr: SocketAddr) -> io::Result<usize> {
    // SAFETY: zeroed() produces valid initialization for SOCKADDR_STORAGE
    let mut storage: WS::SOCKADDR_STORAGE = unsafe { mem::zeroed() };
    let addr_len = encode_sockaddr(addr, &mut storage);
    // SAFETY: sendto with valid fd, data pointer, and sockaddr
    // Returns number of bytes sent; SOCKET_ERROR (-1) on failure.
    let ret = unsafe {
        WS::sendto(
            fd,
            data.as_ptr().cast(),
            data.len() as i32,
            0,
            &storage as *const _ as *const WS::SOCKADDR,
            addr_len,
        )
    };
    if ret != WS::SOCKET_ERROR {
        Ok(ret as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn raw_send(fd: Fd, data: &[u8]) -> io::Result<usize> {
    // SAFETY: send with valid fd and data pointer, for connected sockets
    // Returns number of bytes sent; SOCKET_ERROR (-1) on failure.
    let ret = unsafe { WS::send(fd, data.as_ptr().cast(), data.len() as i32, 0) };
    if ret != WS::SOCKET_ERROR {
        Ok(ret as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn raw_recvfrom(fd: Fd, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
    // SAFETY: zeroed() produces valid initialization for SOCKADDR_STORAGE
    let mut storage: WS::SOCKADDR_STORAGE = unsafe { mem::zeroed() };
    let mut addr_len = mem::size_of::<WS::SOCKADDR_STORAGE>() as i32;
    // SAFETY: recvfrom with valid fd, buf pointer, and sockaddr output
    // Returns number of bytes received; SOCKET_ERROR (-1) on failure.
    let ret = unsafe {
        WS::recvfrom(
            fd,
            buf.as_mut_ptr().cast(),
            buf.len() as i32,
            0,
            &mut storage as *mut _ as *mut WS::SOCKADDR,
            &mut addr_len,
        )
    };
    if ret != WS::SOCKET_ERROR {
        Ok((ret as usize, decode_sockaddr(&storage, addr_len)))
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn raw_getsockopt(fd: Fd, level: i32, optname: i32) -> io::Result<i32> {
    let mut val: i32 = 0;
    let mut len = mem::size_of_val(&val) as i32;
    // SAFETY: getsockopt with valid fd, level, optname, and output pointers
    let ret =
        unsafe { WS::getsockopt(fd, level, optname, &mut val as *mut _ as *mut u8, &mut len) };
    if ret == 0 {
        Ok(val)
    } else {
        Err(io::Error::last_os_error())
    }
}
