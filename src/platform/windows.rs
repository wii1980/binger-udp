//! Windows platform backend — batch UDP I/O via WSASendMsg / WSARecvMsg.
//!
//! Windows has no native batch UDP syscall; we loop individual
//! WSASendMsg / WSARecvMsg calls.  WSASendMsg is exported directly
//! from ws2_32.dll; WSARecvMsg is an extension function that must be
//! loaded at runtime via WSAIoctl + SIO_GET_EXTENSION_FUNCTION_POINTER.

use std::io;
use std::mem;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::OnceLock;

use windows_sys::Win32::Networking::WinSock as WS;

use crate::batch::{RecvBatchRaw, SendBatchRaw};
use crate::sys::Fd;

// ==================================================================
//  Socket-address helpers
// ==================================================================

fn encode_addr_into(addr: SocketAddr, storage: &mut WS::SOCKADDR_STORAGE, namelen: &mut i32) {
    match addr {
        SocketAddr::V4(v4) => {
            let sin = WS::SOCKADDR_IN {
                sin_family: WS::AF_INET as u16,
                sin_port: v4.port().to_be(),
                sin_addr: WS::IN_ADDR {
                    S_un: WS::IN_ADDR_0 {
                        S_addr: u32::from_ne_bytes(v4.ip().octets()),
                    },
                },
                sin_zero: [0i8; 8],
            };
            unsafe {
                *(storage as *mut _ as *mut WS::SOCKADDR_IN) = sin;
            }
            *namelen = mem::size_of::<WS::SOCKADDR_IN>() as i32;
        }
        SocketAddr::V6(v6) => {
            let sin6 = WS::SOCKADDR_IN6 {
                sin6_family: WS::AF_INET6 as u16,
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
            unsafe {
                *(storage as *mut _ as *mut WS::SOCKADDR_IN6) = sin6;
            }
            *namelen = mem::size_of::<WS::SOCKADDR_IN6>() as i32;
        }
    }
}

fn decode_sockaddr(storage: &WS::SOCKADDR_STORAGE, namelen: i32) -> SocketAddr {
    if namelen == 0 {
        return SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
    }
    match storage.ss_family as i32 {
        x if x == WS::AF_INET as i32 => {
            let sin: &WS::SOCKADDR_IN =
                unsafe { &*(storage as *const _ as *const WS::SOCKADDR_IN) };
            let ip = Ipv4Addr::from(u32::from_be(unsafe { sin.sin_addr.S_un.S_addr }));
            let port = u16::from_be(sin.sin_port);
            SocketAddr::V4(SocketAddrV4::new(ip, port))
        }
        x if x == WS::AF_INET6 as i32 => {
            let sin6: &WS::SOCKADDR_IN6 =
                unsafe { &*(storage as *const _ as *const WS::SOCKADDR_IN6) };
            // SAFETY: union field access for sin6_addr.u.Byte and Anonymous
            let ip = Ipv6Addr::from(unsafe { sin6.sin6_addr.u.Byte });
            let port = u16::from_be(sin6.sin6_port);
            SocketAddr::V6(SocketAddrV6::new(ip, port, sin6.sin6_flowinfo, unsafe {
                sin6.Anonymous.sin6_scope_id
            }))
        }
        _ => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
    }
}

// ==================================================================
//  WSARecvMsg function-pointer loading
// ==================================================================

type WsaRecvMsgFn = unsafe extern "system" fn(
    WS::SOCKET,
    *mut WS::WSAMSG,
    *mut u32,
    *const std::ffi::c_void,
    *const std::ffi::c_void,
) -> i32;

static WSARECVMSG_PTR: OnceLock<Option<WsaRecvMsgFn>> = OnceLock::new();

fn get_wsa_recvmsg() -> Option<WsaRecvMsgFn> {
    *WSARECVMSG_PTR.get_or_init(|| {
        let s = unsafe { WS::socket(WS::AF_INET as i32, WS::SOCK_DGRAM as i32, 0) };
        if s == WS::INVALID_SOCKET {
            return None;
        }

        let guid = WS::WSAID_WSARECVMSG;
        let mut func_ptr: Option<WsaRecvMsgFn> = None;
        let mut bytes_returned: u32 = 0;

        let rc = unsafe {
            WS::WSAIoctl(
                s,
                WS::SIO_GET_EXTENSION_FUNCTION_POINTER,
                &guid as *const _ as *const std::ffi::c_void,
                mem::size_of_val(&guid) as u32,
                &mut func_ptr as *mut _ as *mut std::ffi::c_void,
                mem::size_of::<Option<WsaRecvMsgFn>>() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
                None,
            )
        };
        // SAFETY: temporary socket is no longer needed.
        unsafe {
            WS::closesocket(s);
        }

        if rc == WS::SOCKET_ERROR {
            None
        } else {
            func_ptr
        }
    })
}

// ==================================================================
//  try_send_batch
// ==================================================================

pub(crate) fn try_send_batch(fd: Fd, batch: &SendBatchRaw) -> io::Result<usize> {
    let len = batch.len();
    if len == 0 {
        return Ok(0);
    }

    let mut sent = 0usize;
    for i in 0..len {
        let (data, addr) = batch.entry(i);

        let mut wsa_buf = WS::WSABUF {
            len: data.len() as u32,
            buf: data.as_ptr() as *mut u8,
        };

        let mut addr_storage: WS::SOCKADDR_STORAGE = unsafe { mem::zeroed() };
        let mut namelen = 0i32;

        if let Some(target) = addr {
            encode_addr_into(target, &mut addr_storage, &mut namelen);
        }

        let wsa_msg = WS::WSAMSG {
            name: if addr.is_some() {
                &mut addr_storage as *mut _ as *mut _
            } else {
                std::ptr::null_mut()
            },
            namelen,
            lpBuffers: &mut wsa_buf,
            dwBufferCount: 1,
            Control: WS::WSABUF {
                len: 0,
                buf: std::ptr::null_mut(),
            },
            dwFlags: 0,
        };

        let mut bytes_sent: u32 = 0;
        let rc =
            unsafe { WS::WSASendMsg(fd, &wsa_msg, 0, &mut bytes_sent, std::ptr::null_mut(), None) };

        if rc == 0 {
            sent += 1;
        } else {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                break;
            }
            if sent > 0 {
                return Ok(sent);
            }
            return Err(err);
        }
    }
    Ok(sent)
}

// ==================================================================
//  try_recv_batch
// ==================================================================

pub(crate) fn try_recv_batch(fd: Fd, batch: &mut RecvBatchRaw) -> io::Result<usize> {
    let capacity = batch.capacity();
    if capacity == 0 {
        return Ok(0);
    }

    let mut received = 0usize;
    for i in 0..capacity {
        // Split borrow: get buf pointer and length without holding a reference.
        let (buf_ptr, buf_len) = {
            let (buf, _) = batch.buffer_mut(i);
            (buf.as_mut_ptr(), buf.len())
        };

        let mut wsa_buf = WS::WSABUF {
            len: buf_len as u32,
            buf: buf_ptr,
        };

        let mut source: WS::SOCKADDR_STORAGE = unsafe { mem::zeroed() };

        let mut wsa_msg = WS::WSAMSG {
            name: &mut source as *mut _ as *mut _,
            namelen: mem::size_of::<WS::SOCKADDR_STORAGE>() as i32,
            lpBuffers: &mut wsa_buf,
            dwBufferCount: 1,
            Control: WS::WSABUF {
                len: 0,
                buf: std::ptr::null_mut(),
            },
            dwFlags: 0,
        };

        let mut bytes_recv: u32 = 0;

        let result = if let Some(wsa_recvmsg) = get_wsa_recvmsg() {
            let rc = unsafe {
                wsa_recvmsg(
                    fd,
                    &mut wsa_msg,
                    &mut bytes_recv,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            if rc == WS::SOCKET_ERROR {
                Err(io::Error::last_os_error())
            } else {
                Ok(bytes_recv as usize)
            }
        } else {
            let mut addr_len = mem::size_of::<WS::SOCKADDR_STORAGE>() as i32;
            let rc = unsafe {
                WS::recvfrom(
                    fd,
                    buf_ptr as *mut u8,
                    buf_len as i32,
                    0,
                    &mut source as *mut _ as *mut _,
                    &mut addr_len,
                )
            };
            if rc == WS::SOCKET_ERROR {
                Err(io::Error::last_os_error())
            } else {
                Ok(rc as usize)
            }
        };

        match result {
            Ok(n) => {
                let decoded = decode_sockaddr(&source, 0);
                // SAFETY: i < capacity, n <= buf_len
                unsafe { batch.set_recv_len(i, n) };
                let (_, addr_out) = batch.buffer_mut(i);
                *addr_out = decoded;
                batch.set_len(i + 1);
                received += 1;
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    Ok(received)
}
