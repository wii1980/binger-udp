use std::io;
use std::mem;

use crate::batch::{RecvBatchRaw, SendBatchRaw};
use crate::sockaddr;
use crate::sys::Fd;

/// macOS private API struct — equivalent to Linux's mmsghdr but as an extended msghdr.
/// Layout from Apple XNU socket_private.h (64 bytes on 64-bit):
///   msg_name(8) + msg_namelen(4+4pad) + msg_iov(8) + msg_iovlen(4+4pad)
///   + msg_control(8) + msg_controllen(4+4pad) + msg_flags(4+4pad) + msg_datalen(8)
#[repr(C)]
struct msghdr_x {
    msg_name: *mut libc::c_void,
    msg_namelen: libc::socklen_t,
    msg_iov: *mut libc::iovec,
    msg_iovlen: libc::c_int,
    msg_control: *mut libc::c_void,
    msg_controllen: libc::socklen_t,
    msg_flags: libc::c_int,
    msg_datalen: usize,
}

type SendmsgXFn =
    unsafe extern "C" fn(libc::c_int, *const msghdr_x, libc::c_uint, libc::c_int) -> isize;

type RecvmsgXFn =
    unsafe extern "C" fn(libc::c_int, *mut msghdr_x, libc::c_uint, libc::c_int) -> isize;

fn resolve_symbol(lock: &std::sync::OnceLock<usize>, name: &std::ffi::CStr) -> Option<usize> {
    let addr =
        *lock.get_or_init(|| unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) as usize });
    (addr != 0).then_some(addr)
}

fn sendmsg_x_fn() -> Option<SendmsgXFn> {
    static ADDR: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    static NAME: &std::ffi::CStr = unsafe {
        std::ffi::CStr::from_bytes_with_nul_unchecked(b"sendmsg_x\0")
    };
    resolve_symbol(&ADDR, NAME)
        .map(|addr| unsafe { mem::transmute::<usize, SendmsgXFn>(addr) })
}

fn recvmsg_x_fn() -> Option<RecvmsgXFn> {
    static ADDR: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    static NAME: &std::ffi::CStr = unsafe {
        std::ffi::CStr::from_bytes_with_nul_unchecked(b"recvmsg_x\0")
    };
    resolve_symbol(&ADDR, NAME)
        .map(|addr| unsafe { mem::transmute::<usize, RecvmsgXFn>(addr) })
}

fn retry_eintr<F: Fn() -> isize>(f: F) -> io::Result<isize> {
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

pub(crate) fn try_send_batch(fd: Fd, batch: &SendBatchRaw) -> io::Result<usize> {
    let len = batch.len();
    if len == 0 {
        return Ok(0);
    }

    let mut msgs: Vec<msghdr_x> = Vec::with_capacity(len);
    let mut iovecs: Vec<libc::iovec> = Vec::with_capacity(len);
    let mut addrs: Vec<libc::sockaddr_storage> = Vec::with_capacity(len);

    for i in 0..len {
        let (data, addr) = batch.entry(i);
        // SAFETY: zeroed() produces valid initialization for C structs with all-zero being valid
        let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
        // SAFETY: zeroed() produces valid initialization for msghdr_x
        let mut mhdr: msghdr_x = unsafe { mem::zeroed() };

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

        msgs.push(mhdr);
    }

    let mut addr_idx = 0usize;
    for (i, msg) in msgs.iter_mut().enumerate() {
        msg.msg_iov = &iovecs[i] as *const _ as *mut libc::iovec;
        let (_, addr) = batch.entry(i);
        if addr.is_some() {
            msg.msg_name = &addrs[addr_idx] as *const _ as *mut libc::c_void;
            addr_idx += 1;
        }
    }

    if let Some(sendmsg_x) = sendmsg_x_fn() {
        // SAFETY: sendmsg_x is called with valid fd, properly aligned msghdr_x array,
        //         and valid count. Returns count of messages sent (not bytes).
        let sent = retry_eintr(|| unsafe { sendmsg_x(fd, msgs.as_ptr(), len as libc::c_uint, 0) })?;
        return Ok(sent as usize);
    }

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

pub(crate) fn try_recv_batch(fd: Fd, batch: &mut RecvBatchRaw) -> io::Result<usize> {
    let capacity = batch.capacity();
    if capacity == 0 {
        return Ok(0);
    }

    // SAFETY: zeroed() is valid for sockaddr_storage initialization
    let mut addrs: Vec<libc::sockaddr_storage> =
        (0..capacity).map(|_| unsafe { mem::zeroed() }).collect();

    let mut msgs: Vec<msghdr_x> = Vec::with_capacity(capacity);
    let mut iovecs: Vec<libc::iovec> = Vec::with_capacity(capacity);

    for (i, addr_slot) in addrs.iter_mut().enumerate() {
        let (buf, _) = batch.buffer_mut(i);
        let iov = libc::iovec {
            iov_base: buf.as_mut_ptr() as *mut libc::c_void,
            iov_len: buf.len(),
        };

        // SAFETY: zeroed() produces valid initialization for msghdr_x.
        // macOS 10.15 has a bug where it doesn't set msg_controllen, so zero-initialize.
        let mut mhdr: msghdr_x = unsafe { mem::zeroed() };
        mhdr.msg_name = addr_slot as *mut _ as *mut libc::c_void;
        mhdr.msg_namelen = mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        mhdr.msg_iovlen = 1;

        iovecs.push(iov);
        msgs.push(mhdr);
    }

    for (i, msg) in msgs.iter_mut().enumerate() {
        msg.msg_iov = &iovecs[i] as *const _ as *mut libc::iovec;
    }

    if let Some(recvmsg_x) = recvmsg_x_fn() {
        // SAFETY: recvmsg_x with valid fd, properly aligned msghdr_x array, valid capacity.
        //         Returns count of messages (NOT bytes); each message's bytes is in msg_datalen.
        let received = retry_eintr(|| unsafe {
            recvmsg_x(fd, msgs.as_mut_ptr(), capacity as libc::c_uint, 0)
        })?;
        let n = received as usize;
        for i in 0..n {
            let recv_len = msgs[i].msg_datalen;
            let decoded_addr = sockaddr::decode_sockaddr(&addrs[i], msgs[i].msg_namelen);
            // SAFETY: i < n <= capacity, recv_len <= buf.len()
            unsafe { batch.set_recv_len(i, recv_len) };
            let (_, addr_out) = batch.buffer_mut(i);
            *addr_out = decoded_addr;
        }
        batch.set_len(n);
        return Ok(n);
    }

    let mut received = 0;
    for i in 0..batch.capacity() {
        let result = {
            let (buf, _) = batch.buffer_mut(i);
            sockaddr::raw_recvfrom(fd, buf)
        };
        match result {
            Ok((n, addr)) => {
                // SAFETY: i < capacity, n <= buf.len()
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
