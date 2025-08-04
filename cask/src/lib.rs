//! the idea here is:
//! The data is zero copy.
//! The header has a bit of overhead - because it needs to be updated on write.

use std::{collections::VecDeque, sync::Arc};

use bytemuck::Pod;
use bytes::{BufMut, BytesMut};
use parking_lot::Mutex;
use tokio::sync::{Semaphore, SemaphorePermit};

#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Debug, Clone, Copy)]
pub struct BufHeader {
    pub capacity: usize, // Maximum capacity of the buffer in bytes
    pub len: usize,      // Current length of the buffer in bytes
}

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct Buffer<T> {
    inner: BytesMut,
    phantom: std::marker::PhantomData<T>,
}

impl<T: Pod> Buffer<T> {
    const HEADER_SIZE: usize = std::mem::size_of::<BufHeader>();

    pub fn with_capacity(capacity: usize) -> Self {
        let data_size = capacity * std::mem::size_of::<T>();

        let mut buffer = BytesMut::with_capacity(Self::HEADER_SIZE + data_size);
        // buffer.resize(Self::HEADER_SIZE + data_size, 0); // <-- unsure if this is how it should
        let header = BufHeader {
            capacity: data_size,
            len: 0,
        };
        // put the header
        buffer.put(bytemuck::bytes_of(&header));
        // put the data
        let data = vec![0u8; data_size];
        buffer.put(bytes::Bytes::from(data));

        Buffer {
            inner: buffer,
            phantom: std::marker::PhantomData,
        }
    }

    pub fn header(&self) -> &BufHeader {
        bytemuck::checked::from_bytes(&self.inner[..Self::HEADER_SIZE])
    }

    pub fn header_mut(&mut self) -> &mut BufHeader {
        bytemuck::checked::from_bytes_mut(&mut self.inner[..Self::HEADER_SIZE])
    }

    pub fn data(&self) -> &[T] {
        let raw = &self.inner[Self::HEADER_SIZE..];
        bytemuck::checked::cast_slice(raw)
    }

    /// returns the whole buffer of data, which may be uninitialized (0)
    pub fn raw_data_mut(&mut self) -> &mut [T] {
        let raw = &mut self.inner[Self::HEADER_SIZE..];
        bytemuck::checked::cast_slice_mut(raw)
    }

    /// returns the slice of data that is initialized (up to the current length)
    pub fn data_mut(&mut self) -> &mut [T] {
        let (header, data) = self.raw_split_mut();
        &mut data[..header.len]
    }

    pub fn split(&self) -> (&BufHeader, &[T]) {
        let header = self.header();
        let data = self.data();
        (header, data)
    }

    // splits and returns the header and the raw data slice
    pub fn raw_split_mut(&mut self) -> (&mut BufHeader, &mut [T]) {
        let (header_bytes, data_bytes) = self.inner.split_at_mut(Self::HEADER_SIZE);

        let header = bytemuck::checked::from_bytes_mut::<BufHeader>(header_bytes);
        let data = bytemuck::checked::cast_slice_mut::<u8, T>(data_bytes);

        (header, data)
    }

    /// splits and returns the slice of data that is initialized (up to the current length)
    pub fn split_mut(&mut self) -> (&mut BufHeader, &mut [T]) {
        let (header, data) = self.raw_split_mut();
        let data_up_to_len = &mut data[..header.len];
        (header, data_up_to_len)
    }

    pub fn push(&mut self, item: T) {
        let (header, data) = self.raw_split_mut();
        if header.len < header.capacity / std::mem::size_of::<T>() {
            data[header.len] = item;
            header.len += 1;
        } else {
            panic!("Buffer is full, cannot push more items");
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        let (header, data) = self.raw_split_mut();
        if header.len > 0 {
            header.len -= 1;
            Some(data[header.len])
        } else {
            None
        }
    }

    pub fn is_empty(&self) -> bool {
        self.header().len == 0
    }

    pub fn is_full(&self) -> bool {
        let header = self.header();
        header.len == header.capacity / std::mem::size_of::<T>()
    }

    pub fn resize(&mut self, new_size: usize) {
        let (header, data) = self.raw_split_mut();
        if new_size > header.capacity / std::mem::size_of::<T>() {
            panic!("Cannot resize buffer to a larger size than its capacity");
        }
        header.len = new_size;
        data.fill(T::zeroed());
    }

    pub fn copy_from_slice(&mut self, src: &[T]) {
        let (header, data) = self.raw_split_mut();
        if src.len() > header.capacity / std::mem::size_of::<T>() {
            panic!("Source slice is too large for the buffer");
        }
        header.len = src.len();
        data[..src.len()].copy_from_slice(src);
        data[src.len()..].fill(T::zeroed());
    }

    pub fn clear(&mut self) {
        let (header, data) = self.raw_split_mut();
        header.len = 0;
        data.fill(T::zeroed());
    }

    pub fn len(&self) -> usize {
        self.header().len
    }

    pub fn capacity(&self) -> usize {
        self.header().capacity
    }
}

pub struct BufferPool<T> {
    buffers: Arc<Mutex<VecDeque<Buffer<T>>>>,
    capacity: usize,
    buffer_size: usize,
    available: Semaphore,
}

pub struct BufWithPermit<'a, T> {
    permit: SemaphorePermit<'a>,
    buffer: Buffer<T>,
}

impl<T: Pod> BufWithPermit<'_, T> {
    pub fn buffer_mut(&mut self) -> &mut Buffer<T> {
        &mut self.buffer
    }
    pub fn buffer(&self) -> &Buffer<T> {
        &self.buffer
    }
}

impl<T: Pod> BufferPool<T> {
    pub fn empty(buffer_size: usize) -> Self {
        Self {
            buffers: Arc::new(Mutex::new(VecDeque::new())),
            available: Semaphore::new(0),
            capacity: 0,
            buffer_size,
        }
    }

    /// Preallocates a set amount of buffers of size buffer_size * sizeof(T).
    pub fn with_capacity(c: usize, buffer_size: usize) -> Self {
        let mut buffers = VecDeque::with_capacity(c);
        for _ in 0..c {
            let buffer = Buffer::with_capacity(buffer_size);
            buffers.push_back(buffer);
        }

        Self {
            buffers: Arc::new(Mutex::new(buffers)),
            available: Semaphore::new(c),
            capacity: c,
            buffer_size,
        }
    }

    /// Pushes a new buffer into the pool and increments the available permits.
    pub fn push_buffer(&mut self, buffer: Buffer<T>) {
        let mut buffers = self.buffers.lock();
        buffers.push_back(buffer);
        self.available.add_permits(1);
        self.capacity += 1;
    }

    pub fn expand_by(&mut self, additional: usize) {
        let mut buffers = self.buffers.lock();
        for _ in 0..additional {
            buffers.push_back(Buffer::with_capacity(self.buffer_size));
        }
        self.available.add_permits(additional);
        self.capacity += additional;
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn available_buffers(&self) -> usize {
        self.available.available_permits()
    }

    pub fn try_get_buffer(&self) -> Result<BufWithPermit<T>, tokio::sync::TryAcquireError> {
        let maybe_permit = self.available.try_acquire();

        match maybe_permit {
            Ok(permit) => {
                let mut buffers = self.buffers.lock();
                let buffer = buffers
                    .pop_front()
                    .expect("Buffer pool should not be empty");
                Ok(BufWithPermit { permit, buffer })
            }
            Err(e) => Err(e),
        }
    }

    pub async fn get_buffer(&self) -> BufWithPermit<T> {
        let permit = self.available.acquire().await.unwrap();
        let mut buffers = self.buffers.lock();
        let buffer = buffers
            .pop_front()
            .expect("Buffer pool should not be empty");
        BufWithPermit { permit, buffer }
    }

    pub fn return_buffer(&self, buffer_and_permit: BufWithPermit<T>) {
        let mut buffers = self.buffers.lock();
        let mut buffer = buffer_and_permit.buffer;
        let permit = buffer_and_permit.permit;
        buffer.clear();
        buffers.push_back(buffer);
        drop(permit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::{Pod, Zeroable};
    use std::time::Duration;
    use tokio::time::timeout;

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
    struct Sample(u32);

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
    struct AudioSample {
        left: f32,
        right: f32,
    }

    // Basic Buffer Tests
    #[test]
    fn test_buffer_creation_and_access() {
        let mut buffer = Buffer::<Sample>::with_capacity(4);
        assert_eq!(buffer.capacity(), 4 * std::mem::size_of::<Sample>());
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());

        let (header, data) = buffer.raw_split_mut();
        assert_eq!(header.len, 0);
        assert_eq!(header.capacity, 4 * std::mem::size_of::<Sample>());
        assert_eq!(data.len(), 4);
        let (header, data) = buffer.split_mut();
        assert_eq!(header.len, 0);
        assert_eq!(header.capacity, 4 * std::mem::size_of::<Sample>());
        assert_eq!(data.len(), 0); // Only the initialized part of the buffer (0)

        // Manually set data to test split functionality
        header.len = 2;
        let (_, data) = buffer.split_mut();
        assert_eq!(data.len(), 2);
        data[0] = Sample(10);
        data[1] = Sample(20);

        let (header_read, data_read) = buffer.split();
        assert_eq!(header_read.len, 2);
        assert_eq!(data_read[0], Sample(10));
        assert_eq!(data_read[1], Sample(20));
    }

    #[test]
    fn test_buffer_push_pop() {
        let mut buffer = Buffer::<Sample>::with_capacity(3);

        // Test push
        buffer.push(Sample(10));
        buffer.push(Sample(20));
        buffer.push(Sample(30));

        assert_eq!(buffer.len(), 3);
        assert!(buffer.is_full());
        assert!(!buffer.is_empty());

        let data = buffer.data();
        assert_eq!(data[0], Sample(10));
        assert_eq!(data[1], Sample(20));
        assert_eq!(data[2], Sample(30));

        // Test pop
        assert_eq!(buffer.pop(), Some(Sample(30)));
        assert_eq!(buffer.pop(), Some(Sample(20)));
        assert_eq!(buffer.len(), 1);

        assert_eq!(buffer.pop(), Some(Sample(10)));
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());

        assert_eq!(buffer.pop(), None);
    }

    #[test]
    #[should_panic(expected = "Buffer is full, cannot push more items")]
    fn test_buffer_push_overflow() {
        let mut buffer = Buffer::<Sample>::with_capacity(1);
        buffer.push(Sample(10));
        buffer.push(Sample(20)); // Should panic
    }

    #[test]
    fn test_buffer_resize() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);
        buffer.push(Sample(1));
        buffer.push(Sample(2));
        buffer.push(Sample(3));

        buffer.resize(2);
        assert_eq!(buffer.len(), 2);

        buffer.resize(4);
        assert_eq!(buffer.len(), 4);
        // After resize, new elements should be zeroed
        let data = buffer.data();
        assert_eq!(data[0], Sample(0)); // Should be zeroed due to resize
        assert_eq!(data[1], Sample(0));
    }

    #[test]
    #[should_panic(expected = "Cannot resize buffer to a larger size than its capacity")]
    fn test_buffer_resize_overflow() {
        let mut buffer = Buffer::<Sample>::with_capacity(2);
        buffer.resize(3); // Should panic
    }

    #[test]
    fn test_buffer_copy_from_slice() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);
        let source = [Sample(10), Sample(20), Sample(30)];

        buffer.copy_from_slice(&source);
        assert_eq!(buffer.len(), 3);

        let data = buffer.data();
        assert_eq!(data[0], Sample(10));
        assert_eq!(data[1], Sample(20));
        assert_eq!(data[2], Sample(30));
    }

    #[test]
    #[should_panic(expected = "Source slice is too large for the buffer")]
    fn test_buffer_copy_from_slice_overflow() {
        let mut buffer = Buffer::<Sample>::with_capacity(2);
        let source = [Sample(10), Sample(20), Sample(30)];
        buffer.copy_from_slice(&source); // Should panic
    }

    #[test]
    fn test_buffer_clear() {
        let mut buffer = Buffer::<Sample>::with_capacity(3);
        buffer.push(Sample(10));
        buffer.push(Sample(20));

        assert_eq!(buffer.len(), 2);
        buffer.clear();
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_buffer_with_different_types() {
        let mut buffer = Buffer::<AudioSample>::with_capacity(2);
        let sample1 = AudioSample {
            left: 0.5,
            right: -0.3,
        };
        let sample2 = AudioSample {
            left: 0.8,
            right: 0.1,
        };

        buffer.push(sample1);
        buffer.push(sample2);

        assert_eq!(buffer.len(), 2);
        let data = buffer.data();
        assert_eq!(data[0], sample1);
        assert_eq!(data[1], sample2);
    }

    // Buffer Pool Tests
    #[test]
    fn test_buffer_pool_creation() {
        let pool = BufferPool::<Sample>::with_capacity(3, 8);
        assert_eq!(pool.capacity(), 3);
        assert_eq!(pool.available_buffers(), 3);
    }

    #[test]
    fn test_empty_buffer_pool() {
        let pool = BufferPool::<Sample>::empty(1);
        assert_eq!(pool.capacity(), 0);
        assert_eq!(pool.available_buffers(), 0);
    }

    #[tokio::test]
    async fn test_get_and_return_buffer() {
        let pool = BufferPool::<Sample>::with_capacity(1, 2);
        let mut buf_with_permit = pool.get_buffer().await;

        // Use the buffer
        let buffer = buf_with_permit.buffer_mut();
        buffer.push(Sample(123));
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.data()[0], Sample(123));

        // Pool should have no available buffers now
        assert_eq!(pool.available_buffers(), 0);

        // Return the buffer
        pool.return_buffer(buf_with_permit);
        assert_eq!(pool.available_buffers(), 1);
    }

    #[tokio::test]
    async fn test_multiple_buffers() {
        let pool = BufferPool::<Sample>::with_capacity(3, 4);

        let buf1 = pool.get_buffer().await;
        let buf2 = pool.get_buffer().await;
        let buf3 = pool.get_buffer().await;

        assert_eq!(pool.available_buffers(), 0);

        pool.return_buffer(buf1);
        assert_eq!(pool.available_buffers(), 1);

        pool.return_buffer(buf2);
        pool.return_buffer(buf3);
        assert_eq!(pool.available_buffers(), 3);
    }

    #[test]
    fn test_expand_pool() {
        let mut pool = BufferPool::<Sample>::with_capacity(2, 4);
        assert_eq!(pool.capacity(), 2);
        assert_eq!(pool.available_buffers(), 2);

        pool.expand_by(3);
        assert_eq!(pool.capacity(), 5);
        assert_eq!(pool.available_buffers(), 5);
    }

    #[test]
    fn test_try_get_buffer_success() {
        let pool = BufferPool::<Sample>::with_capacity(1, 2);
        let result = pool.try_get_buffer();
        assert!(result.is_ok());
        assert_eq!(pool.available_buffers(), 0);
    }

    #[test]
    fn test_try_get_buffer_failure() {
        let pool = BufferPool::<Sample>::with_capacity(0, 2);
        let result = pool.try_get_buffer();
        assert!(result.is_err());
        assert_eq!(pool.available_buffers(), 0);
    }

    #[test]
    fn test_push_buffer_to_pool() {
        let mut pool = BufferPool::<Sample>::empty(1);
        assert_eq!(pool.capacity(), 0);

        let buffer = Buffer::<Sample>::with_capacity(4);
        pool.push_buffer(buffer);

        assert_eq!(pool.capacity(), 1);
        assert_eq!(pool.available_buffers(), 1);
    }

    #[tokio::test]
    async fn test_buffer_cleared_on_return() {
        let pool = BufferPool::<Sample>::with_capacity(1, 3);

        // Get buffer and add data
        let mut buf_with_permit = pool.get_buffer().await;
        let buffer = buf_with_permit.buffer_mut();
        buffer.push(Sample(100));
        buffer.push(Sample(200));
        assert_eq!(buffer.len(), 2);

        // Return buffer
        pool.return_buffer(buf_with_permit);

        // Get buffer again - should be cleared
        let buf_with_permit = pool.get_buffer().await;
        let buffer = buf_with_permit.buffer();
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }

    #[tokio::test]
    async fn test_concurrent_buffer_access() {
        let pool = Arc::new(BufferPool::<Sample>::with_capacity(2, 4));
        let pool1 = Arc::clone(&pool);
        let pool2 = Arc::clone(&pool);

        let handle1 = tokio::spawn(async move {
            let buf = pool1.get_buffer().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
            pool1.return_buffer(buf);
        });

        let handle2 = tokio::spawn(async move {
            let buf = pool2.get_buffer().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
            pool2.return_buffer(buf);
        });

        let _ = tokio::join!(handle1, handle2);
        assert_eq!(pool.available_buffers(), 2);
    }

    #[tokio::test]
    async fn test_pool_exhaustion_and_waiting() {
        let pool = BufferPool::<Sample>::with_capacity(1, 2);

        // Take the only buffer
        let buf1 = pool.get_buffer().await;
        assert_eq!(pool.available_buffers(), 0);

        // Try to get another buffer with timeout - should timeout
        let result = timeout(Duration::from_millis(100), pool.get_buffer()).await;
        assert!(result.is_err()); // Should timeout

        // Return the buffer
        pool.return_buffer(buf1);

        // Now we should be able to get a buffer immediately
        let buf2 = pool.get_buffer().await;
        assert_eq!(pool.available_buffers(), 0);
        pool.return_buffer(buf2);
    }

    #[test]
    fn test_buffer_memory_layout() {
        let buffer = Buffer::<Sample>::with_capacity(3);
        let header_size = std::mem::size_of::<BufHeader>();
        let data_size = 3 * std::mem::size_of::<Sample>();

        // Verify the internal buffer has the expected size
        assert_eq!(buffer.inner.len(), header_size + data_size);

        // Verify header is at the beginning
        let header = buffer.header();
        assert_eq!(header.capacity, data_size);
        assert_eq!(header.len, 0);
    }

    #[test]
    fn test_large_buffer() {
        let large_size = 1000;
        let mut buffer = Buffer::<Sample>::with_capacity(large_size);

        // Fill half the buffer
        for i in 0..large_size / 2 {
            buffer.push(Sample(i as u32));
        }

        assert_eq!(buffer.len(), large_size / 2);
        assert!(!buffer.is_full());

        // Verify data
        let data = buffer.data();
        for i in 0..large_size / 2 {
            assert_eq!(data[i], Sample(i as u32));
        }
    }

    #[tokio::test]
    async fn test_buffer_reuse_maintains_capacity() {
        let pool = BufferPool::<Sample>::with_capacity(1, 5);

        // Get buffer, use it, return it
        let mut buf1 = pool.get_buffer().await;
        buf1.buffer_mut().push(Sample(1));
        buf1.buffer_mut().push(Sample(2));
        pool.return_buffer(buf1);

        // Get buffer again - should have same capacity but be empty
        let buf2 = pool.get_buffer().await;
        let buffer = buf2.buffer();
        assert_eq!(buffer.capacity(), 5 * std::mem::size_of::<Sample>());
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }
}
