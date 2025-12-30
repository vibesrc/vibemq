//! Buffer pool for reducing allocation overhead
//!
//! Provides reusable BytesMut buffers to avoid repeated allocation/deallocation
//! in hot paths like packet encoding.

use bytes::BytesMut;
use crossbeam_queue::ArrayQueue;
use std::sync::Arc;

/// Default buffer size for pooled buffers
const DEFAULT_BUFFER_SIZE: usize = 2048;

/// Maximum number of buffers to keep in the pool
const MAX_POOLED_BUFFERS: usize = 256;

/// Maximum buffer size to return to pool (don't pool oversized buffers)
const MAX_POOLED_BUFFER_SIZE: usize = 16384;

/// A pool of reusable BytesMut buffers
pub struct BufferPool {
    pool: ArrayQueue<BytesMut>,
    buffer_size: usize,
}

impl BufferPool {
    /// Create a new buffer pool with default settings
    pub fn new() -> Self {
        Self {
            pool: ArrayQueue::new(MAX_POOLED_BUFFERS),
            buffer_size: DEFAULT_BUFFER_SIZE,
        }
    }

    /// Create a new buffer pool with custom buffer size
    pub fn with_buffer_size(buffer_size: usize) -> Self {
        Self {
            pool: ArrayQueue::new(MAX_POOLED_BUFFERS),
            buffer_size,
        }
    }

    /// Get a buffer from the pool, or allocate a new one if pool is empty
    #[inline]
    pub fn get(&self) -> BytesMut {
        self.pool
            .pop()
            .unwrap_or_else(|| BytesMut::with_capacity(self.buffer_size))
    }

    /// Return a buffer to the pool for reuse
    /// Buffer is cleared before being added to pool
    /// Oversized buffers are dropped instead of pooled
    #[inline]
    pub fn put(&self, mut buf: BytesMut) {
        // Don't pool oversized buffers - let them be deallocated
        if buf.capacity() <= MAX_POOLED_BUFFER_SIZE {
            buf.clear();
            // If pool is full, buffer is simply dropped
            let _ = self.pool.push(buf);
        }
    }

    /// Get the number of buffers currently in the pool
    pub fn len(&self) -> usize {
        self.pool.len()
    }

    /// Check if the pool is empty
    pub fn is_empty(&self) -> bool {
        self.pool.is_empty()
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Global buffer pool instance
static GLOBAL_POOL: std::sync::OnceLock<Arc<BufferPool>> = std::sync::OnceLock::new();

/// Get or initialize the global buffer pool
pub fn global_pool() -> &'static Arc<BufferPool> {
    GLOBAL_POOL.get_or_init(|| Arc::new(BufferPool::new()))
}

/// Get a buffer from the global pool
#[inline]
pub fn get_buffer() -> BytesMut {
    global_pool().get()
}

/// Return a buffer to the global pool
#[inline]
pub fn put_buffer(buf: BytesMut) {
    global_pool().put(buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_pool_basic() {
        let pool = BufferPool::new();

        // Get a buffer
        let buf = pool.get();
        assert!(buf.capacity() >= DEFAULT_BUFFER_SIZE);

        // Return it
        pool.put(buf);
        assert_eq!(pool.len(), 1);

        // Get it back
        let buf2 = pool.get();
        assert!(buf2.is_empty()); // Should be cleared
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_buffer_pool_oversized() {
        let pool = BufferPool::new();

        // Create an oversized buffer
        let mut buf = BytesMut::with_capacity(MAX_POOLED_BUFFER_SIZE + 1);
        buf.extend_from_slice(&[0u8; 100]);

        // Return it - should be dropped, not pooled
        pool.put(buf);
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_global_pool() {
        let buf = get_buffer();
        assert!(buf.capacity() >= DEFAULT_BUFFER_SIZE);
        put_buffer(buf);
    }
}
