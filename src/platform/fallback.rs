use std::io;

use crate::batch::{RecvBatchRaw, SendBatchRaw};
use crate::sockaddr;

pub(crate) fn try_send_batch(fd: crate::sys::Fd, batch: &SendBatchRaw) -> io::Result<usize> {
    let len = batch.len();
    if len == 0 {
        return Ok(0);
    }

    let connected = sockaddr::is_connected(fd);
    let mut sent = 0;
    for i in 0..len {
        let (data, addr) = batch.entry(i);
        let result = match (connected, addr) {
            (true, _) | (_, None) => sockaddr::raw_send(fd, data),
            (_, Some(a)) => sockaddr::raw_sendto(fd, data, a),
        };
        match result {
            Ok(_) => sent += 1,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    Ok(sent)
}

pub(crate) fn try_recv_batch(fd: crate::sys::Fd, batch: &mut RecvBatchRaw) -> io::Result<usize> {
    let mut received = 0;
    for i in 0..batch.capacity() {
        let result = sockaddr::raw_recvfrom(fd, batch.buffer_mut(i).0);
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
