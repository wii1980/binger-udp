use std::io;
use std::net::SocketAddr;

use crate::batch::{RecvBatchRaw, SendBatchRaw};

pub(crate) fn try_send_batch(
    fd: std::os::fd::RawFd,
    batch: &SendBatchRaw,
) -> io::Result<usize> {
    let mut sent = 0;
    for i in 0..batch.len() {
        let (data, addr) = batch.entry(i);
        let result = match addr {
            Some(addr) => sendto(fd, data, addr),
            None => send(fd, data),
        };
        match result {
            Ok(n) => {
                debug_assert_eq!(n, data.len());
                sent += 1;
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                break;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(sent)
}

pub(crate) fn try_recv_batch(
    fd: std::os::fd::RawFd,
    batch: &mut RecvBatchRaw,
) -> io::Result<usize> {
    let mut received = 0;
    for i in 0..batch.capacity() {
        let (buf, addr_out) = batch.buffer_mut(i);
        match recvfrom(fd, buf, addr_out) {
            Ok(n) => {
                batch.set_len(i + 1);
                unsafe { batch.set_recv_len(i, n) };
                received += 1;
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                break;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(received)
}

fn sendto(fd: i32, data: &[u8], addr: SocketAddr) -> io::Result<usize> {
    let (addr_ptr, addr_len) = sockaddr_from(addr);
    let ret = unsafe {
        libc::sendto(
            fd,
            data.as_ptr().cast(),
            data.len(),
            0,
            addr_ptr,
            addr_len,
        )
    };
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn send(fd: i32, data: &[u8]) -> io::Result<usize> {
    let ret = unsafe {
        libc::send(fd, data.as_ptr().cast(), data.len(), 0)
    };
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn recvfrom(fd: i32, buf: &mut [u8], addr_out: &mut SocketAddr) -> io::Result<usize> {
    let (mut storage, mut socklen) = sockaddr_storage();
    let ret = unsafe {
        libc::recvfrom(
            fd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            0,
            &mut storage as *mut _ as *mut libc::sockaddr,
            &mut socklen,
        )
    };
    if ret >= 0 {
        *addr_out = sockaddr_into(&storage, socklen);
        Ok(ret as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn sockaddr_from(addr: SocketAddr) -> (*const libc::sockaddr, libc::socklen_t) {
    match addr {
        SocketAddr::V4(v4) => {
            let raw = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: v4.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(v4.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            let ptr = &raw as *const _ as *const libc::sockaddr;
            let len = std::mem::size_of_val(&raw) as libc::socklen_t;
            (ptr, len)
        }
        SocketAddr::V6(v6) => {
            let raw = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: v6.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                },
                sin6_scope_id: v6.scope_id(),
            };
            let ptr = &raw as *const _ as *const libc::sockaddr;
            let len = std::mem::size_of_val(&raw) as libc::socklen_t;
            (ptr, len)
        }
    }
}

fn sockaddr_storage() -> (libc::sockaddr_storage, libc::socklen_t) {
    let storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let len = std::mem::size_of_val(&storage) as libc::socklen_t;
    (storage, len)
}

fn sockaddr_into(storage: &libc::sockaddr_storage, len: libc::socklen_t) -> SocketAddr {
    if len == 0 {
        return SocketAddr::V4(std::net::SocketAddrV4::new(
            std::net::Ipv4Addr::UNSPECIFIED,
            0,
        ));
    }
    match storage.ss_family as i32 {
        libc::AF_INET => {
            let sin: &libc::sockaddr_in = unsafe { &*(storage as *const _ as *const _) };
            let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            let port = u16::from_be(sin.sin_port);
            SocketAddr::V4(std::net::SocketAddrV4::new(ip, port))
        }
        libc::AF_INET6 => {
            let sin6: &libc::sockaddr_in6 = unsafe { &*(storage as *const _ as *const _) };
            let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
            let port = u16::from_be(sin6.sin6_port);
            let flowinfo = sin6.sin6_flowinfo;
            let scope_id = sin6.sin6_scope_id;
            SocketAddr::V6(std::net::SocketAddrV6::new(ip, port, flowinfo, scope_id))
        }
        _ => SocketAddr::V4(std::net::SocketAddrV4::new(
            std::net::Ipv4Addr::UNSPECIFIED,
            0,
        )),
    }
}
