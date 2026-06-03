use std::io;
use std::mem;

use crate::batch::{RecvBatchRaw, SendBatchRaw};
use crate::sockaddr;
use crate::sys;
use crate::sys::Fd;

/// Maximum number of messages to process on the stack without heap fallback.
const MAX_STACK: usize = 64;

/// Size of the cmsg ancillary-data buffer per receive slot.
const CMSG_BUF_SIZE: usize = 256;

/// Retry a closure-returned syscall on `EINTR` (interrupted by signal).
fn retry_eintr<F: FnMut() -> isize>(mut f: F) -> io::Result<isize> {
    loop {
        let n = f();
        if n >= 0 {
            return Ok(n);
        }
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(err);
    }
}

/// Send a batch of UDP datagrams, using `sendmmsg` on the fast path.
///
/// Falls back to per-packet `sendto`/`send` when `ENOSYS` is returned
/// (e.g. older kernels or non-Linux platforms that still reach this code).
pub(crate) fn try_send_batch(fd: Fd, batch: &SendBatchRaw) -> io::Result<usize> {
    let len = batch.len();
    if len == 0 {
        return Ok(0);
    }

    if len <= MAX_STACK {
        // SAFETY: zeroed() is valid for all-zero-bit-pattern C structs
        // (mmsghdr, iovec, sockaddr_storage are all POD).
        let mut msgs: [libc::mmsghdr; MAX_STACK] = unsafe { mem::zeroed() };
        let mut iovecs: [libc::iovec; MAX_STACK] = unsafe { mem::zeroed() };
        let mut addrs: [libc::sockaddr_storage; MAX_STACK] = unsafe { mem::zeroed() };
        let mut addr_idx = 0usize;

        for i in 0..len {
            let (data, addr) = batch.entry(i);

            if let Some(target) = addr {
                // SAFETY: addrs[addr_idx] is an uninitialised but valid
                // zero-initialised sockaddr_storage slot.
                let addr_len = sockaddr::encode_sockaddr(target, &mut addrs[addr_idx]);
                msgs[i].msg_hdr.msg_name = &mut addrs[addr_idx] as *mut _ as *mut libc::c_void;
                msgs[i].msg_hdr.msg_namelen = addr_len;
                addr_idx += 1;
            }

            // SAFETY: data.as_ptr() is valid for the lifetime of the syscall.
            iovecs[i] = libc::iovec {
                iov_base: data.as_ptr() as *mut libc::c_void,
                iov_len: data.len(),
            };

            msgs[i].msg_hdr.msg_iov = &mut iovecs[i] as *mut _;
            msgs[i].msg_hdr.msg_iovlen = 1;
        }

        // SAFETY: sendmmsg with a valid fd, properly aligned and initialised
        // mmsghdr array, and vlen <= MAX_STACK.
        let sent = unsafe { libc::sendmmsg(fd, msgs.as_mut_ptr(), len as u32, 0) };
        if sent >= 0 {
            Ok(sent as usize)
        } else {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ENOSYS) {
                return fallback_send(fd, batch);
            }
            Err(err)
        }
    } else {
        let mut msgs: Vec<libc::mmsghdr> = Vec::with_capacity(len);
        let mut iovecs: Vec<libc::iovec> = Vec::with_capacity(len);
        let mut addrs: Vec<libc::sockaddr_storage> = Vec::with_capacity(len);

        for i in 0..len {
            let (data, addr) = batch.entry(i);
            // SAFETY: zeroed() is valid for all-zero-bit-pattern sockaddr_storage and msghdr.
            let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
            let mut mhdr: libc::msghdr = unsafe { mem::zeroed() };

            if let Some(target) = addr {
                let addr_len = sockaddr::encode_sockaddr(target, &mut storage);
                mhdr.msg_name = &mut storage as *mut _ as *mut libc::c_void;
                mhdr.msg_namelen = addr_len;
            }
            addrs.push(storage);

            let iov = libc::iovec {
                iov_base: data.as_ptr() as *mut libc::c_void,
                iov_len: data.len(),
            };
            iovecs.push(iov);

            mhdr.msg_iov = &iovecs[i] as *const _ as *mut libc::iovec;
            mhdr.msg_iovlen = 1;

            msgs.push(libc::mmsghdr {
                msg_hdr: mhdr,
                msg_len: 0,
            });
        }

        let mut addr_idx = 0usize;
        for (i, msg) in msgs.iter_mut().enumerate() {
            msg.msg_hdr.msg_iov = &iovecs[i] as *const _ as *mut libc::iovec;
            let (_, addr) = batch.entry(i);
            if addr.is_some() {
                msg.msg_hdr.msg_name = &addrs[addr_idx] as *const _ as *mut libc::c_void;
                addr_idx += 1;
            }
        }

        // SAFETY: sendmmsg with valid fd, stable vec backing storage, and valid vlen.
        let sent = unsafe { libc::sendmmsg(fd, msgs.as_mut_ptr(), len as u32, 0) };
        if sent >= 0 {
            Ok(sent as usize)
        } else {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ENOSYS) {
                return fallback_send(fd, batch);
            }
            Err(err)
        }
    }
}

/// Send a single large buffer with GSO (Generic Segmentation Offload).
///
/// Builds a `msghdr` with a `UDP_SEGMENT` cmsg so the kernel fragments the
/// buffer into `segment_size`-byte UDP datagrams.  Returns the number of bytes
/// sent (which may be less than `data.len()` on short sends).
///
/// Requires `sendmsg` support; will fail on kernels without `UDP_SEGMENT`.
#[allow(dead_code)]
pub(crate) fn try_send_gso(fd: Fd, data: &[u8], segment_size: u16) -> io::Result<usize> {
    // SAFETY: zeroed() for POD C structs is valid.
    let mut cmsg_buf: [u8; 64] = unsafe { mem::zeroed() };
    let mut mhdr: libc::msghdr = unsafe { mem::zeroed() };

    mhdr.msg_control = cmsg_buf.as_mut_ptr() as *mut _;
    mhdr.msg_controllen = 64;

    let iov = libc::iovec {
        iov_base: data.as_ptr() as *mut _,
        iov_len: data.len(),
    };
    mhdr.msg_iov = &iov as *const _ as *mut libc::iovec;
    mhdr.msg_iovlen = 1;

    // SAFETY: CMSG_FIRSTHDR is valid on a zeroed msghdr with msg_control set.
    // 64 bytes >= CMSG_SPACE(sizeof(u16)) which is ~24 bytes on x86_64.
    let cm = unsafe { libc::CMSG_FIRSTHDR(&mhdr) };
    if cm.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "CMSG_FIRSTHDR returned null",
        ));
    }
    unsafe {
        (*cm).cmsg_level = sys::IPPROTO_UDP;
        (*cm).cmsg_type = sys::UDP_SEGMENT;
        (*cm).cmsg_len = libc::CMSG_LEN(mem::size_of::<u16>() as libc::c_uint) as _;
        *(libc::CMSG_DATA(cm) as *mut u16) = segment_size;
    }

    // SAFETY: sendmsg with valid fd, fully initialised msghdr including GSO cmsg.
    let ret = unsafe { libc::sendmsg(fd, &mhdr, 0) };
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Receive a batch of UDP datagrams, using `recvmmsg` on the fast path.
///
/// `recvmmsg` is wrapped in a `retry_eintr` loop so that signal interruptions
/// are handled automatically.
///
/// When the **`gro`** feature is enabled, coalesced GRO segments are split
/// into individual `RecvBatchRaw` slots.
///
/// When the **`timestamping`** feature is enabled, `SCM_TIMESTAMPNS` cmsgs are
/// parsed and stored per-slot.
///
/// When the **`pktinfo`** feature is enabled, `IP_PKTINFO` / `IPV6_PKTINFO`
/// cmsgs are parsed and stored per-slot.
///
/// Falls back to per-packet `recvfrom` on `ENOSYS`.
#[allow(clippy::too_many_lines)]
pub(crate) fn try_recv_batch(fd: Fd, batch: &mut RecvBatchRaw) -> io::Result<usize> {
    let capacity = batch.capacity();
    if capacity == 0 {
        return Ok(0);
    }

    if capacity <= MAX_STACK {
        // SAFETY: zeroed() for POD C structs is valid.
        let mut msgs: [libc::mmsghdr; MAX_STACK] = unsafe { mem::zeroed() };
        let mut iovecs: [libc::iovec; MAX_STACK] = unsafe { mem::zeroed() };
        let mut addrs: [libc::sockaddr_storage; MAX_STACK] = unsafe { mem::zeroed() };
        let mut cmsg_bufs: [[u8; CMSG_BUF_SIZE]; MAX_STACK] = unsafe { mem::zeroed() };

        for i in 0..capacity {
            let (buf, _) = batch.buffer_mut(i);
            iovecs[i] = libc::iovec {
                iov_base: buf.as_mut_ptr() as *mut libc::c_void,
                iov_len: buf.len(),
            };

            msgs[i].msg_hdr.msg_name = &mut addrs[i] as *mut _ as *mut libc::c_void;
            msgs[i].msg_hdr.msg_namelen =
                mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
            msgs[i].msg_hdr.msg_iov = &mut iovecs[i] as *mut _;
            msgs[i].msg_hdr.msg_iovlen = 1;
            msgs[i].msg_hdr.msg_control = cmsg_bufs[i].as_mut_ptr() as *mut _;
            msgs[i].msg_hdr.msg_controllen = CMSG_BUF_SIZE;
        }

        // SAFETY: recvmmsg with valid fd, properly aligned arrays, and valid
        // capacity.  Wrapped in retry_eintr to handle EINTR transparently.
        let received = match retry_eintr(|| unsafe {
            libc::recvmmsg(
                fd,
                msgs.as_mut_ptr(),
                capacity as u32,
                0,
                std::ptr::null_mut(),
            ) as isize
        }) {
            Ok(n) => n,
            Err(e) if e.raw_os_error() == Some(libc::ENOSYS) => return fallback_recv(fd, batch),
            Err(e) => return Err(e),
        };

        let n = received as usize;
        let mut out_idx = 0usize;

        for i in 0..n {
            let recv_len = msgs[i].msg_len as usize;
            // SAFETY: i < n <= capacity, recv_len <= buf.len().
            unsafe { batch.set_recv_len(out_idx, recv_len) };

            let decoded_addr = sockaddr::decode_sockaddr(&addrs[i], msgs[i].msg_hdr.msg_namelen);
            let (_, addr_out) = batch.buffer_mut(out_idx);
            *addr_out = decoded_addr;

            #[cfg(any(feature = "gro", feature = "timestamping", feature = "pktinfo"))]
            {
                // SAFETY: CMSG_FIRSTHDR is valid on a msghdr returned from
                // recvmmsg; the kernel has written cmsg data into the buffer.
                let mut cmsg_ptr = unsafe { libc::CMSG_FIRSTHDR(&msgs[i].msg_hdr) };
                #[cfg(feature = "gro")]
                let mut gro_seg_size: u16 = 0;

                while !cmsg_ptr.is_null() {
                    // SAFETY: cmsg_ptr points to a valid cmsghdr within the
                    // msghdr (kernel-populated).
                    let ch = unsafe { &*cmsg_ptr };
                    match (ch.cmsg_level as i32, ch.cmsg_type as i32) {
                        #[cfg(feature = "gro")]
                        (lvl, ty) if lvl == sys::IPPROTO_UDP && ty == sys::UDP_GRO => {
                            // SAFETY: CMSG_DATA for a UDP_GRO cmsg contains
                            // a u16 segment size.
                            let data = unsafe { libc::CMSG_DATA(cmsg_ptr) as *const u16 };
                            gro_seg_size = unsafe { *data };
                        }

                        #[cfg(feature = "timestamping")]
                        (lvl, ty) if lvl == sys::SOL_SOCKET && ty == sys::SCM_TIMESTAMPNS => {
                            // SAFETY: CMSG_DATA for SCM_TIMESTAMPNS contains
                            // a struct timespec.
                            let ts_ptr =
                                unsafe { libc::CMSG_DATA(cmsg_ptr) as *const libc::timespec };
                            let ts = unsafe { *ts_ptr };
                            batch.set_timestamp(
                                out_idx,
                                Some(crate::batch::Timestamp {
                                    tv_sec: ts.tv_sec,
                                    tv_nsec: ts.tv_nsec,
                                }),
                            );
                        }

                        #[cfg(feature = "pktinfo")]
                        (lvl, ty) if lvl == sys::IPPROTO_IP && ty == sys::IP_PKTINFO => {
                            // SAFETY: CMSG_DATA for IP_PKTINFO contains a
                            // struct in_pktinfo.
                            let info =
                                unsafe { &*(libc::CMSG_DATA(cmsg_ptr) as *const libc::in_pktinfo) };
                            let ip = std::net::Ipv4Addr::from(u32::from_be(info.ipi_addr.s_addr));
                            batch.set_dst_addr(
                                out_idx,
                                Some(std::net::SocketAddr::V4(std::net::SocketAddrV4::new(ip, 0))),
                            );
                        }

                        #[cfg(feature = "pktinfo")]
                        (lvl, ty) if lvl == sys::IPPROTO_IPV6 && ty == sys::IPV6_PKTINFO => {
                            // SAFETY: CMSG_DATA for IPV6_PKTINFO contains a
                            // struct in6_pktinfo.
                            let info = unsafe {
                                &*(libc::CMSG_DATA(cmsg_ptr) as *const libc::in6_pktinfo)
                            };
                            let ip = std::net::Ipv6Addr::from(info.ipi6_addr.s6_addr);
                            batch.set_dst_addr(
                                out_idx,
                                Some(std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
                                    ip, 0, 0, 0,
                                ))),
                            );
                        }

                        _ => {}
                    }
                    // SAFETY: CMSG_NXTHDR on a valid msghdr + valid current
                    // cmsg pointer advances to the next cmsg.
                    cmsg_ptr = unsafe { libc::CMSG_NXTHDR(&msgs[i].msg_hdr, cmsg_ptr) };
                }

                #[cfg(feature = "gro")]
                if gro_seg_size > 0 {
                    let total_len = msgs[i].msg_len as usize;
                    let first_idx = out_idx;

                    // Copy the coalesced data out so we can freely mutate
                    // batch without borrow-checker conflicts.
                    let coalesced = {
                        let (buf, _) = batch.buffer_mut(first_idx);
                        buf[..total_len].to_vec()
                    };

                    let mut seg_count = 0usize;
                    let mut offset = 0usize;

                    while offset < total_len {
                        let seg_end = (offset + gro_seg_size as usize).min(total_len);
                        let seg_len = seg_end - offset;

                        if seg_count == 0 {
                            // SAFETY: first_idx < capacity, seg_len <= total_len.
                            unsafe { batch.set_recv_len(first_idx, seg_len) };
                        } else {
                            let next_idx = first_idx + seg_count;
                            if next_idx >= batch.capacity() {
                                break;
                            }
                            let (dst_buf, _) = batch.buffer_mut(next_idx);
                            dst_buf[..seg_len].copy_from_slice(&coalesced[offset..seg_end]);
                            // SAFETY: next_idx < capacity, seg_len <= buf.len().
                            unsafe { batch.set_recv_len(next_idx, seg_len) };
                            let (_, next_addr_out) = batch.buffer_mut(next_idx);
                            *next_addr_out = decoded_addr;
                            #[cfg(feature = "timestamping")]
                            {
                                batch.set_timestamp(next_idx, batch.timestamp(first_idx));
                            }
                            #[cfg(feature = "pktinfo")]
                            {
                                batch.set_dst_addr(next_idx, batch.dst_addr(first_idx));
                            }
                        }

                        seg_count += 1;
                        offset = seg_end;
                    }

                    out_idx = first_idx + seg_count;
                    continue;
                }
            }

            out_idx += 1;
        }

        batch.set_len(out_idx);
        Ok(out_idx)
    } else {
        // SAFETY: zeroed() for POD C structs is valid.
        let mut addrs: Vec<libc::sockaddr_storage> =
            (0..capacity).map(|_| unsafe { mem::zeroed() }).collect();

        let mut msgs: Vec<libc::mmsghdr> = Vec::with_capacity(capacity);
        let mut iovecs: Vec<libc::iovec> = Vec::with_capacity(capacity);
        let mut cmsg_bufs: Vec<[u8; CMSG_BUF_SIZE]> = Vec::with_capacity(capacity);
        // SAFETY: zeroed byte arrays are valid.
        cmsg_bufs.resize_with(capacity, || unsafe { mem::zeroed() });

        for (i, addr_slot) in addrs.iter_mut().enumerate() {
            let (buf, _) = batch.buffer_mut(i);
            let iov = libc::iovec {
                iov_base: buf.as_mut_ptr() as *mut libc::c_void,
                iov_len: buf.len(),
            };

            // SAFETY: zeroed() for msghdr is valid.
            let mut mhdr: libc::msghdr = unsafe { mem::zeroed() };
            mhdr.msg_name = addr_slot as *mut _ as *mut libc::c_void;
            mhdr.msg_namelen = mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
            mhdr.msg_iovlen = 1;
            mhdr.msg_control = cmsg_bufs[i].as_mut_ptr() as *mut _;
            mhdr.msg_controllen = CMSG_BUF_SIZE;

            iovecs.push(iov);
            msgs.push(libc::mmsghdr {
                msg_hdr: mhdr,
                msg_len: 0,
            });
        }

        for (i, msg) in msgs.iter_mut().enumerate() {
            msg.msg_hdr.msg_iov = &iovecs[i] as *const _ as *mut libc::iovec;
        }

        // SAFETY: recvmmsg with valid fd, stable vec backing, and valid
        // capacity.  Wrapped in retry_eintr for EINTR handling.
        let received = match retry_eintr(|| unsafe {
            libc::recvmmsg(
                fd,
                msgs.as_mut_ptr(),
                capacity as u32,
                0,
                std::ptr::null_mut(),
            ) as isize
        }) {
            Ok(n) => n,
            Err(e) if e.raw_os_error() == Some(libc::ENOSYS) => return fallback_recv(fd, batch),
            Err(e) => return Err(e),
        };

        let n = received as usize;
        let mut out_idx = 0usize;

        for i in 0..n {
            let recv_len = msgs[i].msg_len as usize;
            // SAFETY: i < n <= capacity, recv_len <= buf.len().
            unsafe { batch.set_recv_len(out_idx, recv_len) };

            let decoded_addr = sockaddr::decode_sockaddr(&addrs[i], msgs[i].msg_hdr.msg_namelen);
            let (_, addr_out) = batch.buffer_mut(out_idx);
            *addr_out = decoded_addr;

            #[cfg(any(feature = "gro", feature = "timestamping", feature = "pktinfo"))]
            {
                let mut cmsg_ptr = unsafe { libc::CMSG_FIRSTHDR(&msgs[i].msg_hdr) };
                #[cfg(feature = "gro")]
                let mut gro_seg_size: u16 = 0;

                while !cmsg_ptr.is_null() {
                    let ch = unsafe { &*cmsg_ptr };
                    match (ch.cmsg_level as i32, ch.cmsg_type as i32) {
                        #[cfg(feature = "gro")]
                        (lvl, ty) if lvl == sys::IPPROTO_UDP && ty == sys::UDP_GRO => {
                            let data = unsafe { libc::CMSG_DATA(cmsg_ptr) as *const u16 };
                            gro_seg_size = unsafe { *data };
                        }

                        #[cfg(feature = "timestamping")]
                        (lvl, ty) if lvl == sys::SOL_SOCKET && ty == sys::SCM_TIMESTAMPNS => {
                            let ts_ptr =
                                unsafe { libc::CMSG_DATA(cmsg_ptr) as *const libc::timespec };
                            let ts = unsafe { *ts_ptr };
                            batch.set_timestamp(
                                out_idx,
                                Some(crate::batch::Timestamp {
                                    tv_sec: ts.tv_sec,
                                    tv_nsec: ts.tv_nsec,
                                }),
                            );
                        }

                        #[cfg(feature = "pktinfo")]
                        (lvl, ty) if lvl == sys::IPPROTO_IP && ty == sys::IP_PKTINFO => {
                            let info =
                                unsafe { &*(libc::CMSG_DATA(cmsg_ptr) as *const libc::in_pktinfo) };
                            let ip = std::net::Ipv4Addr::from(u32::from_be(info.ipi_addr.s_addr));
                            batch.set_dst_addr(
                                out_idx,
                                Some(std::net::SocketAddr::V4(std::net::SocketAddrV4::new(ip, 0))),
                            );
                        }

                        #[cfg(feature = "pktinfo")]
                        (lvl, ty) if lvl == sys::IPPROTO_IPV6 && ty == sys::IPV6_PKTINFO => {
                            let info = unsafe {
                                &*(libc::CMSG_DATA(cmsg_ptr) as *const libc::in6_pktinfo)
                            };
                            let ip = std::net::Ipv6Addr::from(info.ipi6_addr.s6_addr);
                            batch.set_dst_addr(
                                out_idx,
                                Some(std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
                                    ip, 0, 0, 0,
                                ))),
                            );
                        }

                        _ => {}
                    }
                    cmsg_ptr = unsafe { libc::CMSG_NXTHDR(&msgs[i].msg_hdr, cmsg_ptr) };
                }

                #[cfg(feature = "gro")]
                if gro_seg_size > 0 {
                    let total_len = msgs[i].msg_len as usize;
                    let first_idx = out_idx;

                    let coalesced = {
                        let (buf, _) = batch.buffer_mut(first_idx);
                        buf[..total_len].to_vec()
                    };

                    let mut seg_count = 0usize;
                    let mut offset = 0usize;

                    while offset < total_len {
                        let seg_end = (offset + gro_seg_size as usize).min(total_len);
                        let seg_len = seg_end - offset;

                        if seg_count == 0 {
                            unsafe { batch.set_recv_len(first_idx, seg_len) };
                        } else {
                            let next_idx = first_idx + seg_count;
                            if next_idx >= batch.capacity() {
                                break;
                            }
                            let (dst_buf, _) = batch.buffer_mut(next_idx);
                            dst_buf[..seg_len].copy_from_slice(&coalesced[offset..seg_end]);
                            unsafe { batch.set_recv_len(next_idx, seg_len) };
                            let (_, next_addr_out) = batch.buffer_mut(next_idx);
                            *next_addr_out = decoded_addr;
                            #[cfg(feature = "timestamping")]
                            {
                                batch.set_timestamp(next_idx, batch.timestamp(first_idx));
                            }
                            #[cfg(feature = "pktinfo")]
                            {
                                batch.set_dst_addr(next_idx, batch.dst_addr(first_idx));
                            }
                        }

                        seg_count += 1;
                        offset = seg_end;
                    }

                    out_idx = first_idx + seg_count;
                    continue;
                }
            }

            out_idx += 1;
        }

        batch.set_len(out_idx);
        Ok(out_idx)
    }
}

/// Fallback for `try_send_batch` when `sendmmsg` is not available.
fn fallback_send(fd: Fd, batch: &SendBatchRaw) -> io::Result<usize> {
    let mut sent = 0;
    for i in 0..batch.len() {
        let (data, addr) = batch.entry(i);
        let result = match addr {
            Some(a) => sockaddr::raw_sendto(fd, data, a),
            None => sockaddr::raw_send(fd, data),
        };
        match result {
            Ok(_) => sent += 1,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    Ok(sent)
}

/// Fallback for `try_recv_batch` when `recvmmsg` is not available.
fn fallback_recv(fd: Fd, batch: &mut RecvBatchRaw) -> io::Result<usize> {
    let mut received = 0;
    for i in 0..batch.capacity() {
        let result = {
            let (buf, _) = batch.buffer_mut(i);
            sockaddr::raw_recvfrom(fd, buf)
        };
        match result {
            Ok((n, addr)) => {
                // SAFETY: i < capacity, n <= buf.len().
                unsafe { batch.set_recv_len(i, n) };
                let (_, addr_out) = batch.buffer_mut(i);
                *addr_out = addr;
                batch.set_len(i + 1);
                received += 1;
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    Ok(received)
}
