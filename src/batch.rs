use std::net::SocketAddr;

use crate::error::BingerError;

/// A kernel-provided software timestamp for a received UDP datagram.
///
/// Represents the [`libc::timespec`] structure attached to a packet when the
/// `SO_TIMESTAMPNS` socket option is enabled. The timestamp is recorded
/// by the network stack at the moment of reception.
///
/// Only available on Linux with the `timestamping` feature enabled.
///
/// # Fields
///
/// * `tv_sec` — Seconds since the Unix epoch.
/// * `tv_nsec` — Sub-second nanoseconds.
///
/// # Related
///
/// * [`RecvBatch::timestamp`] — retrieves the timestamp for a received packet.
/// * `BingerUdp::enable_timestamping` — enables kernel timestamping on the socket (Linux only).
#[cfg(feature = "timestamping")]
#[derive(Clone, Copy, Debug, Default)]
pub struct Timestamp {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

#[cfg(feature = "timestamping")]
impl Timestamp {
    /// Converts the timestamp to a [`std::time::Duration`] relative to the Unix epoch.
    ///
    /// This is useful for comparing timestamps or computing inter-packet arrival
    /// times. The duration is always non-negative.
    #[must_use]
    pub fn as_duration(&self) -> std::time::Duration {
        std::time::Duration::new(self.tv_sec as u64, self.tv_nsec as u32)
    }
}

/// A fixed-capacity batch of outgoing UDP datagrams.
///
/// The capacity `N` is a compile-time constant, allowing the batch to be
/// allocated on the stack with no heap allocation in the hot path.
///
/// `SendBatch` dereferences to [`SendBatchRaw`] via [`std::ops::DerefMut`], so it can be
/// passed directly to [`BingerUdp::send_batch`](crate::socket::BingerUdp::send_batch)
/// and [`BingerUdp::try_send_batch`](crate::socket::BingerUdp::try_send_batch).
///
/// # Example
///
/// ```rust,no_run
/// use binger_udp::SendBatch;
///
/// let mut batch = SendBatch::<32>::new();
/// let addr = "192.168.1.1:8080".parse().unwrap();
/// batch.push(b"hello", addr).unwrap();
/// assert_eq!(batch.len(), 1);
/// ```
#[allow(clippy::module_name_repetitions)]
pub struct SendBatch<const N: usize> {
    raw: SendBatchRaw,
}

impl<const N: usize> SendBatch<N> {
    /// Creates a new empty send batch with capacity `N`.
    ///
    /// No heap allocation occurs — the internal slot array is pre-allocated
    /// once to the compile-time capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            raw: SendBatchRaw::with_capacity(N),
        }
    }

    /// Pushes a buffer and destination address into the send batch.
    ///
    /// Use this method for connectionless (multi-destination) sends. Each call
    /// adds one datagram to the batch.
    ///
    /// # Errors
    ///
    /// Returns [`BingerError::BatchFull`] if the batch has reached its capacity
    /// of `N` entries.
    ///
    /// # Related
    ///
    /// * [`SendBatch::push_connected`] — push without a destination for
    ///   pre-connected sockets.
    pub fn push(&mut self, buf: &[u8], addr: SocketAddr) -> Result<(), BingerError> {
        self.raw.push(buf, Some(addr))
    }

    /// Pushes a buffer into the send batch for a pre-connected socket.
    ///
    /// Unlike [`SendBatch::push`], this variant omits the destination address
    /// because the socket is already connected. This is required when using
    /// GSO (Generic Segmentation Offload) on Linux.
    ///
    /// # Errors
    ///
    /// Returns [`BingerError::BatchFull`] if the batch has reached its capacity
    /// of `N` entries.
    pub fn push_connected(&mut self, buf: &[u8]) -> Result<(), BingerError> {
        self.raw.push(buf, None)
    }

    /// Returns the number of packets currently in the batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.raw.len()
    }

    /// Returns `true` if the batch contains no packets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the maximum number of packets this batch can hold.
    ///
    /// This is always the compile-time constant `N`.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Removes all packets from the batch, resetting it for reuse.
    ///
    /// After calling `clear`, [`SendBatch::len`] returns `0` and the batch
    /// can be re-filled with new packets. No memory is deallocated.
    pub fn clear(&mut self) {
        self.raw.clear();
    }
}

impl<const N: usize> Default for SendBatch<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> std::ops::Deref for SendBatch<N> {
    type Target = SendBatchRaw;

    /// Delegates to [`SendBatchRaw`].
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl<const N: usize> std::ops::DerefMut for SendBatch<N> {
    /// Delegates to [`SendBatchRaw`].
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.raw
    }
}

/// A fixed-capacity batch for receiving UDP datagrams.
///
/// The capacity `N` is a compile-time constant. Each slot is pre-allocated
/// with a buffer of `buf_size` bytes, so no allocation occurs during receive.
///
/// `RecvBatch` dereferences to [`RecvBatchRaw`] via [`std::ops::DerefMut`], so it can be
/// passed directly to [`BingerUdp::recv_batch`](crate::socket::BingerUdp::recv_batch)
/// and [`BingerUdp::try_recv_batch`](crate::socket::BingerUdp::try_recv_batch).
///
/// # Example
///
/// ```rust,no_run
/// use binger_udp::RecvBatch;
///
/// let mut batch = RecvBatch::<32>::new(2048);
/// // After recv_batch fills the batch:
/// for i in 0..batch.len() {
///     let data = batch.data(i);
///     let addr = batch.addr(i);
/// }
/// ```
#[allow(clippy::module_name_repetitions)]
pub struct RecvBatch<const N: usize> {
    raw: RecvBatchRaw,
}

impl<const N: usize> RecvBatch<N> {
    /// Creates a new empty receive batch with capacity `N`.
    ///
    /// Each of the `N` slots is pre-allocated with a buffer of `buf_size` bytes.
    /// This allocation happens once at construction time; no further allocation
    /// occurs during receive operations.
    ///
    /// `buf_size` should be large enough to hold the largest expected datagram.
    /// Datagrams exceeding `buf_size` will be truncated.
    #[must_use]
    pub fn new(buf_size: usize) -> Self {
        Self {
            raw: RecvBatchRaw::with_capacity(N, buf_size),
        }
    }

    /// Returns the number of packets received in the last batch operation.
    #[must_use]
    pub fn len(&self) -> usize {
        self.raw.len()
    }

    /// Returns `true` if no packets were received in the last batch operation.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the maximum number of packets this batch can hold.
    ///
    /// This is always the compile-time constant `N`.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Returns the data payload of the packet at index `idx`.
    ///
    /// The returned slice length matches the actual received datagram size,
    /// not the buffer capacity.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    #[must_use]
    pub fn data(&self, idx: usize) -> &[u8] {
        self.raw.data(idx)
    }

    /// Returns the source address of the packet at index `idx`.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    #[must_use]
    pub fn addr(&self, idx: usize) -> SocketAddr {
        self.raw.addr(idx)
    }

    /// Removes all received packets, resetting the batch for reuse.
    ///
    /// After `clear`, [`RecvBatch::len`] returns `0`. Internal buffers are
    /// retained and will be overwritten on the next receive.
    pub fn clear(&mut self) {
        self.raw.clear();
    }

    /// Iterates over all received packets, yielding `(data, source_addr)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], SocketAddr)> + '_ {
        (0..self.len()).map(|i| (self.data(i), self.addr(i)))
    }

    /// Returns the kernel-provided software timestamp for the packet at `idx`,
    /// if available.
    ///
    /// Requires the `timestamping` feature and `BingerUdp::enable_timestamping(true)`
    /// to be called on the socket before receiving (Linux only).
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    #[cfg(feature = "timestamping")]
    #[must_use]
    pub fn timestamp(&self, idx: usize) -> Option<Timestamp> {
        self.raw.timestamp(idx)
    }

    /// Returns the destination address (local interface address) for the packet
    /// at `idx`, if available.
    ///
    /// Requires the `pktinfo` feature and `BingerUdp::enable_pktinfo(true)`
    /// to be called on the socket before receiving (Linux only). Useful for servers that
    /// need to know which local address a packet was sent to (e.g., multi-homed
    /// hosts).
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    #[cfg(feature = "pktinfo")]
    #[must_use]
    pub fn dst_addr(&self, idx: usize) -> Option<SocketAddr> {
        self.raw.dst_addr(idx)
    }
}

impl<const N: usize> std::ops::Deref for RecvBatch<N> {
    type Target = RecvBatchRaw;

    /// Delegates to [`RecvBatchRaw`].
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl<const N: usize> std::ops::DerefMut for RecvBatch<N> {
    /// Delegates to [`RecvBatchRaw`].
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

/// A raw send batch with dynamic capacity.
///
/// This is the underlying type behind [`SendBatch<N>`]. It manages a
/// dynamically-allocated slot array. The typed [`SendBatch<N>`] wrapper
/// provides a compile-time capacity via [`std::ops::DerefMut`] to this type.
///
/// Most users should use [`SendBatch<N>`] instead.
pub struct SendBatchRaw {
    slots: Vec<SendSlot>,
    len: usize,
}

impl SendBatchRaw {
    /// Creates a new raw send batch with the given capacity.
    ///
    /// The internal slot vector is pre-allocated to `capacity` elements.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            len: 0,
        }
    }

    /// Pushes a buffer and optional destination address into the raw send batch.
    ///
    /// When `addr` is `None`, the packet will be sent on a connected socket
    /// (equivalent to [`SendBatch::push_connected`]). When `addr` is `Some(...)`,
    /// the packet is sent to that specific address (connectionless mode).
    ///
    /// # Errors
    ///
    /// Returns [`BingerError::BatchFull`] if the batch has reached its capacity.
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

    /// Returns the number of packets currently in the batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the batch contains no packets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the data and optional destination address at index `idx`.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    #[must_use]
    pub fn entry(&self, idx: usize) -> (&[u8], Option<SocketAddr>) {
        let slot = &self.slots[idx];
        let data = unsafe { std::slice::from_raw_parts(slot.data_ptr, slot.data_len) };
        (data, slot.addr)
    }

    /// Removes all packets from the batch, resetting it for reuse.
    ///
    /// No memory is deallocated.
    pub fn clear(&mut self) {
        self.len = 0;
    }
}

struct RecvSlot {
    buf: Vec<u8>,
    addr: SocketAddr,
    recv_len: u16,
    #[cfg(feature = "timestamping")]
    timestamp: Option<Timestamp>,
    #[cfg(feature = "pktinfo")]
    dst_addr: Option<SocketAddr>,
}

/// A raw receive batch with dynamic capacity.
///
/// This is the underlying type behind [`RecvBatch<N>`]. It manages a
/// dynamically-allocated array of receive slots, each with a pre-allocated
/// buffer. The typed [`RecvBatch<N>`] wrapper provides a compile-time capacity
/// via [`std::ops::DerefMut`] to this type.
///
/// Most users should use [`RecvBatch<N>`] instead.
pub struct RecvBatchRaw {
    slots: Vec<RecvSlot>,
    len: usize,
}

impl RecvBatchRaw {
    /// Creates a new raw receive batch with the given capacity and buffer size.
    ///
    /// All `capacity` slots are pre-allocated, each with a buffer of `buf_size`
    /// bytes. This allocation happens once at construction.
    #[must_use]
    pub fn with_capacity(capacity: usize, buf_size: usize) -> Self {
        let slots = (0..capacity)
            .map(|_| RecvSlot {
                buf: vec![0u8; buf_size],
                addr: SocketAddr::from(([0, 0, 0, 0], 0)),
                recv_len: 0,
                #[cfg(feature = "timestamping")]
                timestamp: None,
                #[cfg(feature = "pktinfo")]
                dst_addr: None,
            })
            .collect();
        Self { slots, len: 0 }
    }

    /// Returns the maximum number of packets this batch can hold.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Returns the number of packets received in the last batch operation.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if no packets were received in the last batch operation.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Sets the number of received packets.
    ///
    /// This is called internally by the platform receive functions to record
    /// how many packets were received.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
    }

    /// Sets the actual received byte count for slot `idx`.
    ///
    /// # Safety
    ///
    /// `idx` must be within bounds and `n` must not exceed the buffer size
    /// of the slot at `idx`.
    pub unsafe fn set_recv_len(&mut self, idx: usize, n: usize) {
        self.slots[idx].recv_len = n as u16;
    }

    /// Returns a mutable reference to the buffer and source address of slot `idx`.
    ///
    /// This is used internally by the platform receive functions to populate
    /// the received data and its source address.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    pub fn buffer_mut(&mut self, idx: usize) -> (&mut [u8], &mut SocketAddr) {
        let slot = &mut self.slots[idx];
        (&mut slot.buf, &mut slot.addr)
    }

    /// Returns the data payload of the packet at index `idx`.
    ///
    /// The returned slice length matches the actual received datagram size.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    #[must_use]
    pub fn data(&self, idx: usize) -> &[u8] {
        let slot = &self.slots[idx];
        &slot.buf[..slot.recv_len as usize]
    }

    /// Returns the source address of the packet at index `idx`.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    #[must_use]
    pub fn addr(&self, idx: usize) -> SocketAddr {
        self.slots[idx].addr
    }

    /// Removes all received packets, resetting the batch for reuse.
    ///
    /// All recv lengths are reset to 0 and metadata (timestamps, dst addresses)
    /// are cleared. Internal buffers are retained.
    pub fn clear(&mut self) {
        self.len = 0;
        for slot in &mut self.slots {
            slot.recv_len = 0;
            #[cfg(feature = "timestamping")]
            {
                slot.timestamp = None;
            }
            #[cfg(feature = "pktinfo")]
            {
                slot.dst_addr = None;
            }
        }
    }

    /// Returns the kernel-provided software timestamp for the packet at `idx`,
    /// if available.
    ///
    /// Requires the `timestamping` feature.
    #[cfg(feature = "timestamping")]
    #[must_use]
    pub fn timestamp(&self, idx: usize) -> Option<Timestamp> {
        self.slots[idx].timestamp
    }

    #[cfg(feature = "timestamping")]
    #[allow(dead_code)]
    pub(crate) fn set_timestamp(&mut self, idx: usize, ts: Option<Timestamp>) {
        self.slots[idx].timestamp = ts;
    }

    /// Returns the destination address (local interface address) for the packet
    /// at `idx`, if available.
    ///
    /// Requires the `pktinfo` feature.
    #[cfg(feature = "pktinfo")]
    #[must_use]
    pub fn dst_addr(&self, idx: usize) -> Option<SocketAddr> {
        self.slots[idx].dst_addr
    }

    #[cfg(feature = "pktinfo")]
    #[allow(dead_code)]
    pub(crate) fn set_dst_addr(&mut self, idx: usize, addr: Option<SocketAddr>) {
        self.slots[idx].dst_addr = addr;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    // -----------------------------------------------------------------------
    // SendBatch<N> tests
    // -----------------------------------------------------------------------

    #[test]
    fn send_batch_new_is_empty() {
        let batch = SendBatch::<4>::new();
        assert_eq!(batch.len(), 0);
        assert!(batch.is_empty());
        assert_eq!(batch.capacity(), 4);
    }

    #[test]
    fn send_batch_push_increments_len() {
        let mut batch = SendBatch::<4>::new();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        batch.push(b"hello", addr).unwrap();
        assert_eq!(batch.len(), 1);
        assert!(!batch.is_empty());
    }

    #[test]
    fn send_batch_push_multiple() {
        let mut batch = SendBatch::<4>::new();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        batch.push(b"a", addr).unwrap();
        batch.push(b"b", addr).unwrap();
        batch.push(b"c", addr).unwrap();
        assert_eq!(batch.len(), 3);
        assert_eq!(batch.entry(0).0, b"a");
        assert_eq!(batch.entry(1).0, b"b");
        assert_eq!(batch.entry(2).0, b"c");
    }

    #[test]
    fn send_batch_push_batch_full() {
        let mut batch = SendBatch::<2>::new();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        batch.push(b"a", addr).unwrap();
        batch.push(b"b", addr).unwrap();
        let err = batch.push(b"c", addr).unwrap_err();
        assert!(matches!(err, BingerError::BatchFull { capacity: 2 }));
    }

    #[test]
    fn send_batch_push_connected_addr_is_none() {
        let mut batch = SendBatch::<4>::new();
        batch.push_connected(b"hello").unwrap();
        assert_eq!(batch.len(), 1);
        let (data, addr_opt) = batch.entry(0);
        assert_eq!(data, b"hello");
        assert!(addr_opt.is_none());
    }

    #[test]
    fn send_batch_clear_resets_len() {
        let mut batch = SendBatch::<4>::new();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        batch.push(b"a", addr).unwrap();
        batch.push(b"b", addr).unwrap();
        assert_eq!(batch.len(), 2);
        batch.clear();
        assert_eq!(batch.len(), 0);
        assert!(batch.is_empty());
    }

    #[test]
    fn send_batch_default_trait() {
        let batch: SendBatch<4> = SendBatch::default();
        assert_eq!(batch.len(), 0);
        assert!(batch.is_empty());
        assert_eq!(batch.capacity(), 4);
    }

    #[test]
    fn send_batch_capacity_returns_const_generic() {
        let batch = SendBatch::<16>::new();
        assert_eq!(batch.capacity(), 16);

        let batch_small = SendBatch::<1>::new();
        assert_eq!(batch_small.capacity(), 1);
    }

    #[test]
    fn send_batch_deref_to_raw() {
        let mut batch = SendBatch::<4>::new();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        batch.push(b"data", addr).unwrap();
        let (data, addr_opt) = batch.entry(0);
        assert_eq!(data, b"data");
        assert_eq!(addr_opt, Some(addr));
    }

    // -----------------------------------------------------------------------
    // RecvBatch<N> tests
    // -----------------------------------------------------------------------

    #[test]
    fn recv_batch_new_has_correct_capacity() {
        let batch = RecvBatch::<4>::new(1024);
        assert_eq!(batch.capacity(), 4);
        assert_eq!(batch.len(), 0);
        assert!(batch.is_empty());
    }

    #[test]
    fn recv_batch_data_returns_empty_slice_initial() {
        let batch = RecvBatch::<4>::new(1024);
        assert!(batch.data(0).is_empty());
        assert!(batch.data(1).is_empty());
        assert!(batch.data(2).is_empty());
        assert!(batch.data(3).is_empty());
    }

    #[test]
    fn recv_batch_addr_returns_default_initial() {
        let batch = RecvBatch::<4>::new(1024);
        let default: SocketAddr = ([0, 0, 0, 0], 0).into();
        assert_eq!(batch.addr(0), default);
        assert_eq!(batch.addr(3), default);
    }

    #[test]
    fn recv_batch_simulate_receive() {
        let mut batch = RecvBatch::<4>::new(64);
        let addr: SocketAddr = "127.0.0.1:9000".parse().unwrap();

        let (buf, slot_addr) = batch.buffer_mut(0);
        buf[..5].copy_from_slice(b"hello");
        *slot_addr = addr;
        // Safety: idx=0 is within bounds, 5 <= buf_size=64
        unsafe {
            batch.set_recv_len(0, 5);
        }
        batch.set_len(1);

        assert_eq!(batch.len(), 1);
        assert_eq!(batch.data(0), b"hello");
        assert_eq!(batch.addr(0), addr);
    }

    #[test]
    fn recv_batch_clear_resets_len_and_recv_data() {
        let mut batch = RecvBatch::<4>::new(64);
        let addr: SocketAddr = "127.0.0.1:9000".parse().unwrap();

        let (buf, slot_addr) = batch.buffer_mut(0);
        buf[..5].copy_from_slice(b"hello");
        *slot_addr = addr;
        unsafe {
            batch.set_recv_len(0, 5);
        }
        batch.set_len(1);

        batch.clear();
        assert_eq!(batch.len(), 0);
        assert!(batch.is_empty());
        assert!(batch.data(0).is_empty());
    }

    #[test]
    fn recv_batch_iter_empty() {
        let batch = RecvBatch::<4>::new(64);
        let items: Vec<_> = batch.iter().collect();
        assert!(items.is_empty());
    }

    #[test]
    fn recv_batch_iter_yields_correct_items() {
        let mut batch = RecvBatch::<4>::new(64);
        let addr1: SocketAddr = "127.0.0.1:9001".parse().unwrap();
        let addr2: SocketAddr = "127.0.0.1:9002".parse().unwrap();

        let (buf, slot_addr) = batch.buffer_mut(0);
        buf[..2].copy_from_slice(b"ab");
        *slot_addr = addr1;
        unsafe {
            batch.set_recv_len(0, 2);
        }

        let (buf, slot_addr) = batch.buffer_mut(1);
        buf[..3].copy_from_slice(b"cde");
        *slot_addr = addr2;
        unsafe {
            batch.set_recv_len(1, 3);
        }

        batch.set_len(2);

        let items: Vec<_> = batch.iter().collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], (&b"ab"[..], addr1));
        assert_eq!(items[1], (&b"cde"[..], addr2));
    }

    #[test]
    fn recv_batch_capacity_returns_const_generic() {
        let batch = RecvBatch::<8>::new(512);
        assert_eq!(batch.capacity(), 8);

        let batch_small = RecvBatch::<2>::new(512);
        assert_eq!(batch_small.capacity(), 2);
    }

    // -----------------------------------------------------------------------
    // SendBatchRaw tests
    // -----------------------------------------------------------------------

    #[test]
    fn send_raw_with_capacity_creates_empty() {
        let raw = SendBatchRaw::with_capacity(4);
        assert_eq!(raw.len(), 0);
        assert!(raw.is_empty());
    }

    #[test]
    fn send_raw_push_with_addr() {
        let mut raw = SendBatchRaw::with_capacity(4);
        let addr: SocketAddr = "10.0.0.1:1234".parse().unwrap();
        raw.push(b"test data", Some(addr)).unwrap();
        assert_eq!(raw.len(), 1);

        let (data, addr_opt) = raw.entry(0);
        assert_eq!(data, b"test data");
        assert_eq!(addr_opt, Some(addr));
    }

    #[test]
    fn send_raw_push_without_addr() {
        let mut raw = SendBatchRaw::with_capacity(4);
        raw.push(b"no-destination", None).unwrap();

        let (data, addr_opt) = raw.entry(0);
        assert_eq!(data, b"no-destination");
        assert!(addr_opt.is_none());
    }

    #[test]
    fn send_raw_batch_full() {
        let mut raw = SendBatchRaw::with_capacity(2);
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        raw.push(b"a", Some(addr)).unwrap();
        raw.push(b"b", Some(addr)).unwrap();
        let err = raw.push(b"c", Some(addr)).unwrap_err();
        assert!(matches!(err, BingerError::BatchFull { capacity: 2 }));
    }

    #[test]
    fn send_raw_clear_resets_len() {
        let mut raw = SendBatchRaw::with_capacity(4);
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        raw.push(b"a", Some(addr)).unwrap();
        raw.push(b"b", Some(addr)).unwrap();
        assert_eq!(raw.len(), 2);
        raw.clear();
        assert_eq!(raw.len(), 0);
        assert!(raw.is_empty());
    }

    #[test]
    fn send_raw_after_clear_can_push_again() {
        let mut raw = SendBatchRaw::with_capacity(2);
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        raw.push(b"a", Some(addr)).unwrap();
        raw.push(b"b", Some(addr)).unwrap();
        raw.clear();
        raw.push(b"c", Some(addr)).unwrap();
        assert_eq!(raw.len(), 1);
        assert_eq!(raw.entry(0).0, b"c");
    }

    // -----------------------------------------------------------------------
    // RecvBatchRaw tests
    // -----------------------------------------------------------------------

    #[test]
    fn recv_raw_with_capacity() {
        let raw = RecvBatchRaw::with_capacity(3, 128);
        assert_eq!(raw.capacity(), 3);
        assert_eq!(raw.len(), 0);
        assert!(raw.is_empty());
    }

    #[test]
    fn recv_raw_buffer_mut_returns_slice_and_addr_mut() {
        let mut raw = RecvBatchRaw::with_capacity(2, 64);
        let (buf, addr) = raw.buffer_mut(0);
        assert_eq!(buf.len(), 64);
        assert_eq!(*addr, SocketAddr::from(([0, 0, 0, 0], 0)));

        let new_addr: SocketAddr = "192.168.1.1:9999".parse().unwrap();
        *addr = new_addr;

        let (_, addr_back) = raw.buffer_mut(0);
        assert_eq!(*addr_back, new_addr);
    }

    #[test]
    fn recv_raw_set_recv_len_and_data() {
        let mut raw = RecvBatchRaw::with_capacity(1, 32);
        let (buf, _addr) = raw.buffer_mut(0);
        buf[..4].copy_from_slice(b"data");

        // Safety: idx=0 is within bounds, 4 <= buf_size=32
        unsafe {
            raw.set_recv_len(0, 4);
        }

        assert_eq!(raw.data(0), b"data");
        assert_eq!(raw.data(0).len(), 4);
    }

    #[test]
    fn recv_raw_set_len_updates_len() {
        let mut raw = RecvBatchRaw::with_capacity(4, 32);
        assert_eq!(raw.len(), 0);

        raw.set_len(2);
        assert_eq!(raw.len(), 2);
        assert!(!raw.is_empty());

        raw.set_len(0);
        assert_eq!(raw.len(), 0);
        assert!(raw.is_empty());
    }

    #[test]
    fn recv_raw_clear_resets_len_and_recv_len() {
        let mut raw = RecvBatchRaw::with_capacity(2, 32);
        let addr: SocketAddr = "10.0.0.1:5555".parse().unwrap();

        let (buf, slot_addr) = raw.buffer_mut(0);
        buf[..3].copy_from_slice(b"foo");
        *slot_addr = addr;
        unsafe {
            raw.set_recv_len(0, 3);
        }

        let (buf, slot_addr) = raw.buffer_mut(1);
        buf[..3].copy_from_slice(b"bar");
        *slot_addr = addr;
        unsafe {
            raw.set_recv_len(1, 3);
        }

        raw.set_len(2);

        raw.clear();

        assert_eq!(raw.len(), 0);
        assert!(raw.is_empty());
        assert!(raw.data(0).is_empty());
        assert!(raw.data(1).is_empty());
    }

    #[test]
    fn recv_raw_untouched_slot_data_is_empty() {
        let raw = RecvBatchRaw::with_capacity(4, 32);
        assert!(raw.data(0).is_empty());
        assert!(raw.data(3).is_empty());
    }

    // -----------------------------------------------------------------------
    // Timestamp tests (only when feature "timestamping" is enabled)
    // -----------------------------------------------------------------------

    #[cfg(feature = "timestamping")]
    #[test]
    fn timestamp_as_duration_basic() {
        let ts = Timestamp {
            tv_sec: 1,
            tv_nsec: 500_000_000,
        };
        let dur = ts.as_duration();
        assert_eq!(dur.as_secs(), 1);
        assert_eq!(dur.subsec_nanos(), 500_000_000);
    }

    #[cfg(feature = "timestamping")]
    #[test]
    fn timestamp_as_duration_zero() {
        let ts = Timestamp {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let dur = ts.as_duration();
        assert_eq!(dur.as_secs(), 0);
        assert_eq!(dur.subsec_nanos(), 0);
    }

    #[cfg(feature = "timestamping")]
    #[test]
    fn timestamp_as_duration_large_values() {
        let ts = Timestamp {
            tv_sec: 1_000_000,
            tv_nsec: 123_456_789,
        };
        let dur = ts.as_duration();
        assert_eq!(dur.as_secs(), 1_000_000);
        assert_eq!(dur.subsec_nanos(), 123_456_789);
    }

    #[cfg(feature = "timestamping")]
    #[test]
    fn timestamp_default_is_zero() {
        let ts: Timestamp = Timestamp::default();
        assert_eq!(ts.tv_sec, 0);
        assert_eq!(ts.tv_nsec, 0);
    }

    #[cfg(feature = "timestamping")]
    #[test]
    fn timestamp_clone_copy_debug() {
        let ts = Timestamp {
            tv_sec: 42,
            tv_nsec: 7,
        };
        let cloned = ts;
        assert_eq!(cloned.tv_sec, 42);
        assert_eq!(cloned.tv_nsec, 7);
        let debug_str = format!("{ts:?}");
        assert!(debug_str.contains("42"));
        assert!(debug_str.contains('7'));
    }
}
