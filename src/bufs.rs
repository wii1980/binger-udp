use std::sync::Arc;

use crossbeam_queue::ArrayQueue;

/// A lock-free pool of pre-allocated receive buffers.
///
/// `BufferPool` avoids heap allocation in the hot path by recycling
/// [`bytes::BytesMut`] buffers via a bounded, lock-free queue.
///
/// When the pool is exhausted, [`BufferPool::get`] falls back to allocating a
/// new zeroed buffer — the pool never blocks.
///
/// # Example
///
/// ```rust
/// use udp_binger::BufferPool;
///
/// let pool = BufferPool::new(16, 2048);
/// let mut buf = pool.get();
/// buf.extend_from_slice(b"hello");
/// pool.put(buf);
/// assert_eq!(pool.available(), 1);
/// ```
pub struct BufferPool {
    pool: Arc<ArrayQueue<bytes::BytesMut>>,
    buf_size: usize,
}

impl BufferPool {
    /// Creates a new buffer pool with `capacity` buffers, each of `buf_size`
    /// bytes.
    ///
    /// Buffers are lazily allocated — the constructor itself does not allocate.
    #[must_use]
    pub fn new(capacity: usize, buf_size: usize) -> Self {
        Self {
            pool: Arc::new(ArrayQueue::new(capacity)),
            buf_size,
        }
    }

    /// Retrieves a buffer from the pool, or allocates a new zeroed one if the
    /// pool is empty.
    ///
    /// Returned buffers are cleared before being handed out.
    #[must_use]
    pub fn get(&self) -> bytes::BytesMut {
        match self.pool.pop() {
            Some(mut b) => {
                b.clear();
                b
            }
            None => bytes::BytesMut::zeroed(self.buf_size),
        }
    }

    /// Returns a buffer to the pool for reuse.
    ///
    /// If the pool is full the buffer is silently dropped (the pool never
    /// grows).
    pub fn put(&self, buf: bytes::BytesMut) {
        let _ = self.pool.push(buf);
    }

    /// Number of buffers currently available in the pool.
    #[must_use]
    pub fn available(&self) -> usize {
        self.pool.len()
    }

    /// Maximum number of buffers the pool can hold.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.pool.capacity()
    }
}

impl Clone for BufferPool {
    fn clone(&self) -> Self {
        Self {
            pool: Arc::clone(&self.pool),
            buf_size: self.buf_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_pool_is_empty() {
        let pool = BufferPool::new(4, 64);
        assert_eq!(pool.available(), 0);
        assert_eq!(pool.capacity(), 4);
    }

    #[test]
    fn get_returns_zeroed_buffer_when_empty() {
        let pool = BufferPool::new(4, 64);
        let buf = pool.get();
        assert_eq!(buf.len(), 64);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn put_and_get_roundtrip() {
        let pool = BufferPool::new(4, 64);
        let mut buf = pool.get();
        buf.extend_from_slice(b"hello");
        pool.put(buf);

        assert_eq!(pool.available(), 1);

        let buf2 = pool.get();
        // get() calls clear(), so len is 0 — but capacity is preserved
        assert_eq!(buf2.len(), 0, "buffer should be cleared on get()");
        assert!(buf2.capacity() >= 5, "buffer should retain underlying capacity");
    }

    #[test]
    fn put_does_not_exceed_capacity() {
        let pool = BufferPool::new(2, 64);
        let b1 = pool.get();
        let b2 = pool.get();
        let b3 = pool.get();

        pool.put(b1);
        pool.put(b2);
        pool.put(b3); // silently dropped

        assert_eq!(pool.available(), 2);
    }

    #[test]
    fn clone_shares_pool() {
        let pool = BufferPool::new(4, 64);
        let cloned = pool.clone();

        let buf = pool.get();
        assert_eq!(buf.len(), 64);

        // Cloned pool sees the same state
        assert_eq!(cloned.available(), 0);
        assert_eq!(cloned.capacity(), 4);
    }

    #[test]
    fn concurrent_access() {
        let pool = Arc::new(BufferPool::new(16, 64));
        let mut handles = Vec::new();

        for _ in 0..8 {
            let p = Arc::clone(&pool);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    let buf = p.get();
                    p.put(buf);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert!(pool.available() <= pool.capacity());
    }
}
