use std::sync::Arc;

use crossbeam_queue::ArrayQueue;

pub struct BufferPool {
    pool: Arc<ArrayQueue<bytes::BytesMut>>,
    buf_size: usize,
}

impl BufferPool {
    pub fn new(capacity: usize, buf_size: usize) -> Self {
        Self {
            pool: Arc::new(ArrayQueue::new(capacity)),
            buf_size,
        }
    }

    pub fn get(&self) -> bytes::BytesMut {
        self.pool
            .pop()
            .map(|mut b| {
                b.clear();
                b
            })
            .unwrap_or_else(|| bytes::BytesMut::zeroed(self.buf_size))
    }

    pub fn put(&self, buf: bytes::BytesMut) {
        let _ = self.pool.push(buf);
    }

    pub fn available(&self) -> usize {
        self.pool.len()
    }

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
