use std::net::SocketAddr;

use crate::error::BingerError;

pub struct SendBatch<const N: usize> {
    raw: SendBatchRaw,
}

impl<const N: usize> SendBatch<N> {
    const CAPACITY: usize = N;

    pub fn new() -> Self {
        Self {
            raw: SendBatchRaw::with_capacity(N),
        }
    }

    pub fn push(&mut self, buf: &[u8], addr: SocketAddr) -> Result<(), BingerError> {
        self.raw.push(buf, Some(addr))
    }

    pub fn push_connected(&mut self, buf: &[u8]) -> Result<(), BingerError> {
        self.raw.push(buf, None)
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn clear(&mut self) {
        self.raw.clear()
    }
}

impl<const N: usize> Default for SendBatch<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> std::ops::Deref for SendBatch<N> {
    type Target = SendBatchRaw;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl<const N: usize> std::ops::DerefMut for SendBatch<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.raw
    }
}

pub struct RecvBatch<const N: usize> {
    raw: RecvBatchRaw,
}

impl<const N: usize> RecvBatch<N> {
    const CAPACITY: usize = N;

    pub fn new(buf_size: usize) -> Self {
        Self {
            raw: RecvBatchRaw::with_capacity(N, buf_size),
        }
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn data(&self, idx: usize) -> &[u8] {
        self.raw.data(idx)
    }

    pub fn addr(&self, idx: usize) -> SocketAddr {
        self.raw.addr(idx)
    }

    pub fn clear(&mut self) {
        self.raw.clear()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&[u8], SocketAddr)> + '_ {
        (0..self.len()).map(|i| (self.data(i), self.addr(i)))
    }
}

impl<const N: usize> std::ops::Deref for RecvBatch<N> {
    type Target = RecvBatchRaw;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl<const N: usize> std::ops::DerefMut for RecvBatch<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.raw
    }
}

struct SendSlot {
    data_ptr: *const u8,
    data_len: usize,
    addr: Option<SocketAddr>,
    _marker: std::marker::PhantomData<*const [u8]>,
}

pub(crate) struct SendBatchRaw {
    slots: Vec<SendSlot>,
    len: usize,
}

impl SendBatchRaw {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            len: 0,
        }
    }

    pub fn push(&mut self, data: &[u8], addr: Option<SocketAddr>) -> Result<(), BingerError> {
        if self.len >= self.slots.capacity() {
            return Err(BingerError::BatchFull {
                capacity: self.slots.capacity(),
            });
        }
        let slot = SendSlot {
            data_ptr: data.as_ptr(),
            data_len: data.len(),
            addr,
            _marker: std::marker::PhantomData,
        };
        if self.slots.len() <= self.len {
            self.slots.push(slot);
        } else {
            self.slots[self.len] = slot;
        }
        self.len += 1;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn entry(&self, idx: usize) -> (&[u8], Option<SocketAddr>) {
        let slot = &self.slots[idx];
        let data = unsafe { std::slice::from_raw_parts(slot.data_ptr, slot.data_len) };
        (data, slot.addr)
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

struct RecvSlot {
    buf: Vec<u8>,
    addr: SocketAddr,
    recv_len: u16,
}

pub(crate) struct RecvBatchRaw {
    slots: Vec<RecvSlot>,
    buf_size: usize,
    len: usize,
}

impl RecvBatchRaw {
    pub fn with_capacity(capacity: usize, buf_size: usize) -> Self {
        let slots = (0..capacity)
            .map(|_| RecvSlot {
                buf: vec![0u8; buf_size],
                addr: SocketAddr::from(([0, 0, 0, 0], 0)),
                recv_len: 0,
            })
            .collect();
        Self {
            slots,
            buf_size,
            len: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn set_len(&mut self, len: usize) {
        self.len = len;
    }

    pub unsafe fn set_recv_len(&mut self, idx: usize, n: usize) {
        self.slots[idx].recv_len = n as u16;
    }

    pub fn buffer_mut(&mut self, idx: usize) -> (&mut [u8], &mut SocketAddr) {
        let slot = &mut self.slots[idx];
        (&mut slot.buf, &mut slot.addr)
    }

    pub fn data(&self, idx: usize) -> &[u8] {
        let slot = &self.slots[idx];
        &slot.buf[..slot.recv_len as usize]
    }

    pub fn addr(&self, idx: usize) -> SocketAddr {
        self.slots[idx].addr
    }

    pub fn clear(&mut self) {
        self.len = 0;
        for slot in &mut self.slots {
            slot.recv_len = 0;
        }
    }
}
