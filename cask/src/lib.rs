//! the idea here is:
//! The data is zero copy.
//! The header has a bit of overhead - because it needs to be updated on write.

use std::{collections::VecDeque, sync::Arc};

use bytemuck::Pod;
use bytes::{BufMut, BytesMut};
use parking_lot::Mutex;
use tokio::sync::{Semaphore, SemaphorePermit};

#[repr(C)]
#[derive(bytemuck::Zeroable, Debug, Clone, Copy)]
pub struct BufHeader<T: Pod> {
    pub capacity_bytes: usize,
    pub capacity_t: usize,
    pub len_bytes: usize,
    pub len_t: usize,
    pub _phantom: std::marker::PhantomData<T>,
}

impl<T: Pod> BufHeader<T> {
    pub fn update_length(&mut self, len_t: usize) {
        self.len_t = len_t;
        self.len_bytes = len_t * std::mem::size_of::<T>();
    }

    pub fn update_capacity(&mut self, capacity_t: usize) {
        self.capacity_t = capacity_t;
        self.capacity_bytes = capacity_t * std::mem::size_of::<T>();
    }
}

unsafe impl<T: Pod> Pod for BufHeader<T> {}

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct Buffer<T> {
    inner: BytesMut,
    phantom: std::marker::PhantomData<T>,
}

impl<T: Pod> Buffer<T> {
    const HEADER_SIZE: usize = std::mem::size_of::<BufHeader<T>>();

    fn max_elements(header: &BufHeader<T>) -> usize {
        header.capacity_t
    }

    pub fn with_capacity(capacity_t: usize) -> Self {
        let data_size = capacity_t * std::mem::size_of::<T>();
        let mut buffer = BytesMut::with_capacity(Self::HEADER_SIZE + data_size);

        let header = BufHeader::<T> {
            capacity_bytes: data_size,
            capacity_t,
            len_bytes: 0,
            len_t: 0,
            _phantom: std::marker::PhantomData,
        };

        buffer.put(bytemuck::bytes_of(&header));
        buffer.put(vec![0u8; data_size].as_slice());

        Buffer {
            inner: buffer,
            phantom: std::marker::PhantomData,
        }
    }

    pub fn header(&self) -> &BufHeader<T> {
        bytemuck::checked::from_bytes(&self.inner[..Self::HEADER_SIZE])
    }

    pub fn header_mut(&mut self) -> &mut BufHeader<T> {
        bytemuck::checked::from_bytes_mut(&mut self.inner[..Self::HEADER_SIZE])
    }

    pub fn data(&self) -> &[T] {
        let (header_bytes, data_bytes) = self.inner.split_at(Self::HEADER_SIZE);

        let header = bytemuck::checked::from_bytes::<BufHeader<T>>(header_bytes);
        let data = bytemuck::checked::cast_slice::<u8, T>(data_bytes);

        &data[..header.len_t]
    }

    pub fn raw_data_mut(&mut self) -> &mut [T] {
        let raw = &mut self.inner[Self::HEADER_SIZE..];
        bytemuck::checked::cast_slice_mut(raw)
    }

    pub fn data_mut(&mut self) -> &mut [T] {
        let (header, data) = self.raw_split_mut();
        &mut data[..header.len_t]
    }

    pub fn split(&self) -> (&BufHeader<T>, &[T]) {
        let header = self.header();
        let data = self.data();
        (header, data)
    }

    pub fn raw_split_mut(&mut self) -> (&mut BufHeader<T>, &mut [T]) {
        let (header_bytes, data_bytes) = self.inner.split_at_mut(Self::HEADER_SIZE);

        let header = bytemuck::checked::from_bytes_mut::<BufHeader<T>>(header_bytes);
        let data = bytemuck::checked::cast_slice_mut::<u8, T>(data_bytes);

        (header, data)
    }

    pub fn split_mut(&mut self) -> (&mut BufHeader<T>, &mut [T]) {
        let (header, data) = self.raw_split_mut();
        let data_up_to_len = &mut data[..header.len_t];
        (header, data_up_to_len)
    }

    pub fn push(&mut self, item: T) {
        let (header, data) = self.raw_split_mut();
        let max = Self::max_elements(header);
        if header.len_t < max {
            data[header.len_t] = item;
            header.update_length(header.len_t + 1);
        } else {
            panic!("Buffer is full, cannot push more items");
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        let (header, data) = self.raw_split_mut();
        if header.len_t > 0 {
            let new_len_t = header.len_t - 1;
            header.update_length(new_len_t);
            Some(data[new_len_t])
        } else {
            None
        }
    }

    pub fn is_empty(&self) -> bool {
        self.header().len_t == 0
    }

    pub fn is_full(&self) -> bool {
        let header = self.header();
        header.len_t == header.capacity_t
    }

    /// Does not shrink the total capacity of the buffer,
    /// but resizes the data portion to a new size by updating the header.
    ///
    /// Cannot increase the size beyond the max capacity of the buffer.
    pub fn resize_data(&mut self, new_size: usize) {
        let (header, data) = self.raw_split_mut();
        if new_size > header.capacity_t {
            panic!("Cannot resize buffer to a larger size than its capacity");
        }
        header.update_length(new_size);
        data.fill(T::zeroed());
    }

    pub fn copy_from_slice(&mut self, src: &[T]) {
        let (header, data) = self.raw_split_mut();
        if src.len() > header.capacity_t {
            panic!("Source slice is too large for the buffer");
        }
        data[..src.len()].copy_from_slice(src);
        data[src.len()..].fill(T::zeroed());
        header.update_length(src.len());
    }

    pub fn copy_at_most_n<I: IntoIterator<Item = T>>(&mut self, src: I, n: usize) -> usize {
        let (header, data) = self.raw_split_mut();
        let max_len = data.len().min(n);

        let mut iter = src.into_iter();
        let mut count = 0;

        for slot in data.iter_mut().take(max_len) {
            if let Some(item) = iter.next() {
                *slot = item;
                count += 1;
            } else {
                break;
            }
        }

        for slot in data.iter_mut().skip(count).take(n - count) {
            *slot = T::zeroed();
        }

        header.update_length(count);
        count
    }

    pub fn clear(&mut self) {
        let (header, data) = self.raw_split_mut();
        data.fill(T::zeroed());
        header.update_length(0);
    }

    pub fn len_bytes(&self) -> usize {
        self.header().len_bytes
    }

    pub fn capacity_bytes(&self) -> usize {
        self.header().capacity_bytes
    }

    pub fn len(&self) -> usize {
        self.header().len_t
    }

    pub fn capacity(&self) -> usize {
        self.header().capacity_t
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

    pub fn with_capacity(c: usize, buffer_size: usize) -> Self {
        let mut buffers = VecDeque::with_capacity(c);
        for _ in 0..c {
            buffers.push_back(Buffer::with_capacity(buffer_size));
        }

        Self {
            buffers: Arc::new(Mutex::new(buffers)),
            available: Semaphore::new(c),
            capacity: c,
            buffer_size,
        }
    }

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
        match self.available.try_acquire() {
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
        assert_eq!(buffer.capacity_bytes(), 4 * std::mem::size_of::<Sample>());
        assert_eq!(buffer.len_bytes(), 0);
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());

        let (header, data) = buffer.raw_split_mut();
        assert_eq!(header.len_bytes, 0);
        assert_eq!(header.capacity_bytes, 4 * std::mem::size_of::<Sample>());
        assert_eq!(data.len(), 4);
        let (header, data) = buffer.split_mut();
        assert_eq!(header.len_bytes, 0);
        assert_eq!(header.capacity_bytes, 4 * std::mem::size_of::<Sample>());
        assert_eq!(data.len(), 0); // Only the initialized part of the buffer (0)

        // Manually set data to test split functionality
        header.len_bytes = 2 * std::mem::size_of::<Sample>();
        header.len_t = 2;

        let (_, data) = buffer.split_mut();
        assert_eq!(data.len(), 2);
        data[0] = Sample(10);
        data[1] = Sample(20);

        let (header_read, data_read) = buffer.split();
        assert_eq!(header_read.len_t, 2);
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

        buffer.resize_data(2);
        assert_eq!(buffer.len(), 2);

        buffer.resize_data(4);
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
        buffer.resize_data(3); // Should panic
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
        assert_eq!(buffer.len_bytes(), 0);
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
        assert_eq!(buffer.len_bytes(), 0);
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
        let header_size = std::mem::size_of::<BufHeader<Sample>>();
        let data_size = 3 * std::mem::size_of::<Sample>();

        // Verify the internal buffer has the expected size
        assert_eq!(buffer.inner.len(), header_size + data_size);

        // Verify header is at the beginning
        let header = buffer.header();
        assert_eq!(header.capacity_bytes, data_size);
        assert_eq!(header.len_bytes, 0);
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
        assert_eq!(buffer.capacity_bytes(), 5 * std::mem::size_of::<Sample>());
        assert_eq!(buffer.len_bytes(), 0);
        assert!(buffer.is_empty());
    }
    #[test]
    fn test_capacity_t_and_len_t_on_creation() {
        let buffer = Buffer::<Sample>::with_capacity(5);
        let header = buffer.header();

        assert_eq!(header.capacity_t, 5);
        assert_eq!(header.capacity_bytes, 5 * std::mem::size_of::<Sample>());
        assert_eq!(header.len_t, 0);
        assert_eq!(header.len_bytes, 0);
    }

    #[test]
    fn test_len_t_increases_on_push() {
        let mut buffer = Buffer::<Sample>::with_capacity(3);
        buffer.push(Sample(1));
        buffer.push(Sample(2));

        let header = buffer.header();
        assert_eq!(header.len_t, 2);
        assert_eq!(header.len_bytes, 2 * std::mem::size_of::<Sample>());
    }

    #[test]
    fn test_len_t_decreases_on_pop() {
        let mut buffer = Buffer::<Sample>::with_capacity(3);
        buffer.push(Sample(1));
        buffer.push(Sample(2));
        buffer.pop();

        let header = buffer.header();
        assert_eq!(header.len_t, 1);
        assert_eq!(header.len_bytes, std::mem::size_of::<Sample>());
    }

    #[test]
    fn test_len_t_and_capacity_t_on_resize() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);
        buffer.push(Sample(10));
        buffer.push(Sample(20));
        buffer.push(Sample(30));

        buffer.resize_data(2);
        let header = buffer.header();
        assert_eq!(header.len_t, 2);
        assert_eq!(header.len_bytes, 2 * std::mem::size_of::<Sample>());

        buffer.resize_data(5);
        let header = buffer.header();
        assert_eq!(header.len_t, 5);
        assert_eq!(header.len_bytes, 5 * std::mem::size_of::<Sample>());
    }

    #[test]
    fn test_len_t_on_clear() {
        let mut buffer = Buffer::<Sample>::with_capacity(3);
        buffer.push(Sample(1));
        buffer.push(Sample(2));
        buffer.clear();

        let header = buffer.header();
        assert_eq!(header.len_t, 0);
        assert_eq!(header.len_bytes, 0);
    }

    #[test]
    fn test_copy_from_slice_sets_len_t() {
        let mut buffer = Buffer::<Sample>::with_capacity(4);
        let slice = [Sample(1), Sample(2), Sample(3)];

        buffer.copy_from_slice(&slice);

        let header = buffer.header();
        assert_eq!(header.len_t, 3);
        assert_eq!(header.len_bytes, 3 * std::mem::size_of::<Sample>());
    }

    #[test]
    fn test_copy_at_most_n_sets_len_t() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);
        let vec = vec![Sample(5), Sample(6)];

        let copied = buffer.copy_at_most_n(vec.clone(), 4);

        let header = buffer.header();
        assert_eq!(copied, 2);
        assert_eq!(header.len_t, 2);
        assert_eq!(header.len_bytes, 2 * std::mem::size_of::<Sample>())
    }

    #[tokio::test]
    async fn test_len_t_reset_on_buffer_return() {
        let pool = BufferPool::<Sample>::with_capacity(1, 3);

        let mut buffer_with_permit = pool.get_buffer().await;
        buffer_with_permit.buffer_mut().push(Sample(1));
        assert_eq!(buffer_with_permit.buffer().header().len_t, 1);

        pool.return_buffer(buffer_with_permit);

        let buffer_with_permit = pool.get_buffer().await;
        assert_eq!(buffer_with_permit.buffer().header().len_t, 0);
        assert_eq!(buffer_with_permit.buffer().header().len_bytes, 0);
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    use bytemuck::{Pod, Zeroable};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;
    use tokio::time::timeout;

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
    struct Sample(u32);

    // Additional test types
    #[repr(C)]
    #[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
    struct LargeStruct {
        data: [u64; 16], // 128 bytes
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
    struct ComplexStruct {
        id: u32,
        flags: u16,
        padding: u16,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
    struct SmallStruct(u8);

    // =============================================================================
    // 1. Edge Cases and Boundary Conditions
    // =============================================================================

    #[test]
    fn test_zero_capacity_buffer() {
        let mut buffer = Buffer::<Sample>::with_capacity(0);
        assert_eq!(buffer.capacity(), 0);
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        assert!(buffer.is_full()); // Zero capacity means always full

        // Should not be able to push
        // Note: This will panic, but that's expected behavior
        // buffer.push(Sample(1)); // Would panic

        assert_eq!(buffer.pop(), None);
    }

    #[test]
    fn test_single_element_buffer() {
        let mut buffer = Buffer::<Sample>::with_capacity(1);

        // Initially empty
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());

        // Push one element
        buffer.push(Sample(42));
        assert!(!buffer.is_empty());
        assert!(buffer.is_full());
        assert_eq!(buffer.len(), 1);

        // Pop it back
        assert_eq!(buffer.pop(), Some(Sample(42)));
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
    }

    #[test]
    fn test_alternating_push_pop_patterns() {
        let mut buffer = Buffer::<Sample>::with_capacity(3);

        // Pattern: push, push, pop, push, pop, pop, push
        buffer.push(Sample(1));
        buffer.push(Sample(2));
        assert_eq!(buffer.len(), 2);

        assert_eq!(buffer.pop(), Some(Sample(2)));
        assert_eq!(buffer.len(), 1);

        buffer.push(Sample(3));
        assert_eq!(buffer.len(), 2);

        assert_eq!(buffer.pop(), Some(Sample(3)));
        assert_eq!(buffer.pop(), Some(Sample(1)));
        assert_eq!(buffer.len(), 0);

        buffer.push(Sample(4));
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.data()[0], Sample(4));
    }

    #[test]
    fn test_pop_from_empty_buffer_repeatedly() {
        let mut buffer = Buffer::<Sample>::with_capacity(2);

        // Pop from empty buffer multiple times
        for _ in 0..5 {
            assert_eq!(buffer.pop(), None);
            assert!(buffer.is_empty());
            assert_eq!(buffer.len(), 0);
        }
    }

    #[test]
    fn test_resize_to_zero() {
        let mut buffer = Buffer::<Sample>::with_capacity(3);
        buffer.push(Sample(1));
        buffer.push(Sample(2));

        buffer.resize_data(0);
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        assert_eq!(buffer.data().len(), 0);
    }

    #[test]
    fn test_resize_to_exact_capacity() {
        let mut buffer = Buffer::<Sample>::with_capacity(4);
        buffer.push(Sample(1));
        buffer.push(Sample(2));

        buffer.resize_data(buffer.capacity());
        assert_eq!(buffer.len(), 4);
        assert!(buffer.is_full());

        let data = buffer.data();
        assert_eq!(data[0], Sample(0)); // Should be zeroed
        assert_eq!(data[1], Sample(0)); // Should be zeroed

        assert_eq!(buffer.data().len(), 4);

        // resize to smaller
        buffer.resize_data(2);
        assert_eq!(buffer.len(), 2);
        assert!(!buffer.is_full());
        assert_eq!(buffer.data().len(), 2); // Still has capacity for 4, but only 2 are valid
        assert_eq!(buffer.capacity(), 4); // Capacity remains unchanged
    }

    #[test]
    fn test_multiple_consecutive_resizes() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);
        buffer.push(Sample(10));
        buffer.push(Sample(20));

        // Up
        buffer.resize_data(4);
        assert_eq!(buffer.len(), 4);

        // Down
        buffer.resize_data(1);
        assert_eq!(buffer.len(), 1);

        // Up again
        buffer.resize_data(3);
        assert_eq!(buffer.len(), 3);

        // All new data should be zeroed
        let data = buffer.data();
        assert_eq!(data[0], Sample(0));
        assert_eq!(data[1], Sample(0));
        assert_eq!(data[2], Sample(0));
    }

    // =============================================================================
    // 2. Type Safety and Different Pod Types
    // =============================================================================

    #[test]
    fn test_primitive_types() {
        let mut u8_buffer = Buffer::<u8>::with_capacity(4);
        u8_buffer.push(255);
        assert_eq!(u8_buffer.pop(), Some(255));

        let mut f32_buffer = Buffer::<f32>::with_capacity(4);
        f32_buffer.push(3.14000);
        assert_eq!(f32_buffer.pop(), Some(3.14000));

        let mut u64_buffer = Buffer::<u64>::with_capacity(4);
        u64_buffer.push(u64::MAX);
        assert_eq!(u64_buffer.pop(), Some(u64::MAX));
    }

    #[test]
    fn test_complex_struct() {
        let mut buffer = Buffer::<ComplexStruct>::with_capacity(2);
        let complex = ComplexStruct {
            id: 42,
            flags: 0xFFFF,
            padding: 0,
        };

        buffer.push(complex);
        assert_eq!(buffer.pop(), Some(complex));
    }

    #[test]
    fn test_large_struct() {
        let mut buffer = Buffer::<LargeStruct>::with_capacity(2);
        let large = LargeStruct {
            data: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        };

        buffer.push(large);
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.pop(), Some(large));
    }

    #[test]
    fn test_small_struct() {
        let mut buffer = Buffer::<SmallStruct>::with_capacity(100);

        for i in 0u8..50 {
            buffer.push(SmallStruct(i));
        }

        assert_eq!(buffer.len(), 50);

        for i in (0u8..50).rev() {
            assert_eq!(buffer.pop(), Some(SmallStruct(i)));
        }
    }

    // =============================================================================
    // 3. Memory Layout and Integrity
    // =============================================================================

    #[test]
    fn test_memory_layout_consistency() {
        let buffer = Buffer::<ComplexStruct>::with_capacity(5);
        let header_size = std::mem::size_of::<BufHeader<ComplexStruct>>();
        let data_size = 5 * std::mem::size_of::<ComplexStruct>();

        assert_eq!(buffer.inner.len(), header_size + data_size);

        // Verify header is at correct position
        let header = buffer.header();
        assert_eq!(header.capacity_bytes, data_size);
        assert_eq!(header.capacity_t, 5);
    }

    #[test]
    fn test_data_integrity_after_operations() {
        let mut buffer = Buffer::<ComplexStruct>::with_capacity(3);
        let items = [
            ComplexStruct {
                id: 1,
                flags: 0x0001,
                padding: 0,
            },
            ComplexStruct {
                id: 2,
                flags: 0x0002,
                padding: 0,
            },
            ComplexStruct {
                id: 3,
                flags: 0x0003,
                padding: 0,
            },
        ];

        // Fill buffer
        for &item in &items {
            buffer.push(item);
        }

        // Verify all data is intact
        let data = buffer.data();
        for (i, &expected) in items.iter().enumerate() {
            assert_eq!(data[i], expected);
        }

        // Pop one and verify remaining data is still intact
        buffer.pop();
        let remaining_data = buffer.data();
        for (i, &expected) in items[..2].iter().enumerate() {
            assert_eq!(remaining_data[i], expected);
        }
    }

    // =============================================================================
    // 4. Copy Operations and Edge Cases
    // =============================================================================

    #[test]
    fn test_copy_at_most_n_with_insufficient_data() {
        let mut buffer = Buffer::<Sample>::with_capacity(10);
        let data = vec![Sample(1), Sample(2)]; // Only 2 items

        let copied = buffer.copy_at_most_n(data, 5); // Request 5
        assert_eq!(copied, 2); // Should only copy 2
        assert_eq!(buffer.len(), 2);

        let buffer_data = buffer.data();
        assert_eq!(buffer_data[0], Sample(1));
        assert_eq!(buffer_data[1], Sample(2));
    }

    #[test]
    fn test_copy_at_most_n_with_excess_data() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);
        let data = vec![
            Sample(1),
            Sample(2),
            Sample(3),
            Sample(4),
            Sample(5),
            Sample(6),
        ]; // 6 items

        let copied = buffer.copy_at_most_n(data, 3); // Request only 3
        assert_eq!(copied, 3);
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn test_copy_at_most_n_empty_iterator() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);
        let data: Vec<Sample> = vec![];

        let copied = buffer.copy_at_most_n(data, 3);
        assert_eq!(copied, 0);
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_copy_from_slice_empty_slice() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);
        let empty_slice: &[Sample] = &[];

        buffer.copy_from_slice(empty_slice);
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_copy_from_slice_overwrites_existing_data() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);

        // First, add some data
        buffer.push(Sample(100));
        buffer.push(Sample(200));
        assert_eq!(buffer.len(), 2);

        // Now copy from slice - should overwrite
        let new_data = [Sample(1), Sample(2), Sample(3)];
        buffer.copy_from_slice(&new_data);

        assert_eq!(buffer.len(), 3);
        let data = buffer.data();
        assert_eq!(data[0], Sample(1));
        assert_eq!(data[1], Sample(2));
        assert_eq!(data[2], Sample(3));
    }

    // =============================================================================
    // 5. Buffer Pool Advanced Tests
    // =============================================================================

    //#[test]
    //fn test_empty_pool_operations() {
    //    let mut pool = BufferPool::<Sample>::empty(4);
    //    assert_eq!(pool.capacity(), 0);
    //    assert_eq!(pool.available_buffers(), 0);

    //    // Try to get buffer from empty pool
    //    let result = pool.try_get_buffer();
    //    assert!(result.is_err());

    //    // Add a buffer manually
    //    let buffer = Buffer::with_capacity(4);
    //    pool.push_buffer(buffer);

    //    assert_eq!(pool.capacity(), 1);
    //    assert_eq!(pool.available_buffers(), 1);

    //    // Now should be able to get a buffer
    //    let result = pool.try_get_buffer();
    //    assert!(result.is_ok());
    //}

    //#[tokio::test]
    //async fn test_pool_expansion_during_use() {
    //    let mut pool = BufferPool::<Sample>::with_capacity(1, 4);

    //    // Get the only buffer
    //    let buf1 = pool.get_buffer().await;
    //    assert_eq!(pool.available_buffers(), 0);

    //    // Expand the pool
    //    pool.expand_by(2);
    //    assert_eq!(pool.capacity(), 3);
    //    assert_eq!(pool.available_buffers(), 2);

    //    // Should be able to get another buffer immediately
    //    let buf2 = pool.get_buffer().await;
    //    assert_eq!(pool.available_buffers(), 1);

    //    pool.return_buffer(buf1);
    //    pool.return_buffer(buf2);
    //    assert_eq!(pool.available_buffers(), 3);
    //}

    #[tokio::test]
    async fn test_buffer_pool_with_different_types() {
        let pool1 = BufferPool::<u8>::with_capacity(2, 100);
        let pool2 = BufferPool::<ComplexStruct>::with_capacity(2, 10);

        let buf1 = pool1.get_buffer().await;
        let buf2 = pool2.get_buffer().await;

        // Different pools should work independently
        assert_eq!(pool1.available_buffers(), 1);
        assert_eq!(pool2.available_buffers(), 1);

        pool1.return_buffer(buf1);
        pool2.return_buffer(buf2);
    }

    // =============================================================================
    // 6. Concurrency Tests
    // =============================================================================

    //#[tokio::test]
    //async fn test_high_concurrency_buffer_access() {
    //    let pool = Arc::new(BufferPool::<Sample>::with_capacity(5, 10));
    //    let mut handles = vec![];

    //    // Spawn 20 tasks competing for 5 buffers
    //    for i in 0..20 {
    //        let pool_clone = Arc::clone(&pool);
    //        let handle = tokio::spawn(async move {
    //            let mut buf = pool_clone.get_buffer().await;
    //            buf.buffer_mut().push(Sample(i as u32));

    //            // Simulate some work
    //            tokio::time::sleep(Duration::from_millis(1)).await;

    //            let value = buf.buffer().data()[0];
    //            pool_clone.return_buffer(buf);
    //            value.0
    //        });
    //        handles.push(handle);
    //    }

    //    // Wait for all tasks to complete
    //    let results = futures::future::join_all(handles).await;

    //    // Verify all tasks completed successfully
    //    assert_eq!(results.len(), 20);
    //    for (i, result) in results.iter().enumerate() {
    //        assert_eq!(result.as_ref().unwrap(), &(i as u32));
    //    }

    //    // Pool should be back to full capacity
    //    assert_eq!(pool.available_buffers(), 5);
    //}
    #[tokio::test]
    async fn test_concurrent_pool_expansion() {
        let pool = Arc::new(tokio::sync::Mutex::new(
            BufferPool::<Sample>::with_capacity(1, 5),
        ));

        let pool1 = Arc::clone(&pool);
        let pool2 = Arc::clone(&pool);

        // One task expands the pool
        let expand_handle = tokio::spawn(async move {
            let mut pool = pool1.lock().await;
            pool.expand_by(3);
        });

        // Another task tries to use the pool
        let use_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1)).await; // Give expand task a chance
            let pool = pool2.lock().await;
            let buf = pool.get_buffer().await;
            pool.return_buffer(buf);
        });

        let _ = tokio::join!(expand_handle, use_handle);
    }

    // =============================================================================
    // 7. Stress Tests and Performance Edge Cases
    // =============================================================================

    #[test]
    fn test_large_buffer_operations() {
        let capacity = 10_000;
        let mut buffer = Buffer::<Sample>::with_capacity(capacity);

        // Fill completely
        for i in 0..capacity {
            buffer.push(Sample(i as u32));
        }

        assert!(buffer.is_full());
        assert_eq!(buffer.len(), capacity);

        // Verify data integrity
        let data = buffer.data();
        for i in 0..capacity {
            assert_eq!(data[i], Sample(i as u32));
        }

        // Empty completely
        for i in 0..capacity {
            let expected = Sample((capacity - 1 - i) as u32);
            assert_eq!(buffer.pop(), Some(expected));
        }

        assert!(buffer.is_empty());
    }

    #[test]
    fn test_repeated_fill_and_clear_cycles() {
        let mut buffer = Buffer::<Sample>::with_capacity(100);

        // Perform multiple fill/clear cycles
        for cycle in 0..10 {
            // Fill buffer
            for i in 0..100 {
                buffer.push(Sample((cycle * 100 + i) as u32));
            }

            assert!(buffer.is_full());

            // Clear and verify
            buffer.clear();
            assert!(buffer.is_empty());
            assert_eq!(buffer.len(), 0);
        }
    }

    //#[tokio::test]
    //async fn test_pool_stress_with_timeouts() {
    //    let pool = Arc::new(BufferPool::<Sample>::with_capacity(2, 5));
    //    let success_count = Arc::new(AtomicUsize::new(0));
    //    let timeout_count = Arc::new(AtomicUsize::new(0));

    //    let mut handles = vec![];

    //    // Spawn many tasks with tight timeouts
    //    for _ in 0..50 {
    //        let pool_clone = Arc::clone(&pool);
    //        let success_clone = Arc::clone(&success_count);
    //        let timeout_clone = Arc::clone(&timeout_count);

    //        let handle = tokio::spawn(async move {
    //            match timeout(Duration::from_millis(5), pool_clone.get_buffer()).await {
    //                Ok(buf) => {
    //                    success_clone.fetch_add(1, Ordering::Relaxed);

    //                    // Hold buffer briefly
    //                    tokio::time::sleep(Duration::from_millis(1)).await;
    //                    pool_clone.return_buffer(buf);
    //                }
    //                Err(_) => {
    //                    timeout_clone.fetch_add(1, Ordering::Relaxed);
    //                }
    //            }
    //        });
    //        handles.push(handle);
    //    }

    //    // Wait for all tasks
    //    futures::future::join_all(handles).await;

    //    let successes = success_count.load(Ordering::Relaxed);
    //    let timeouts = timeout_count.load(Ordering::Relaxed);

    //    // Should have both successes and timeouts due to contention
    //    assert!(successes > 0);
    //    assert!(timeouts > 0);
    //    assert_eq!(successes + timeouts, 50);

    //    // Pool should be back to full capacity
    //    assert_eq!(pool.available_buffers(), 2);
    //}

    // =============================================================================
    // 8. Error Handling and Recovery
    // =============================================================================

    //#[tokio::test]
    //async fn test_cancellation_safety() {
    //    let pool = Arc::new(BufferPool::<Sample>::with_capacity(1, 5));

    //    // Get the only buffer
    //    let buf = pool.get_buffer().await;

    //    // Start a task that waits for a buffer but cancel it
    //    let pool_clone = Arc::clone(&pool);
    //    let buf = pool_clone.get_buffer().await;
    //    let waiting_task = tokio::spawn(async move { buf });

    //    // Give the task a moment to start waiting
    //    tokio::time::sleep(Duration::from_millis(10)).await;

    //    // Cancel the waiting task
    //    waiting_task.abort();

    //    // Return the buffer
    //    pool.return_buffer(buf);

    //    // Pool should still be functional
    //    let new_buf = pool.get_buffer().await;
    //    pool.return_buffer(new_buf);

    //    assert_eq!(pool.available_buffers(), 1);
    //}

    //#[tokio::test]
    //async fn test_multiple_waiters_with_single_buffer() {
    //    let pool = Arc::new(BufferPool::<Sample>::with_capacity(1, 5));

    //    // Get the only buffer
    //    let buf = pool.get_buffer().await;

    //    // Start multiple waiters
    //    let mut waiters = vec![];
    //    for i in 0..5 {
    //        let pool_clone = Arc::clone(&pool);
    //        let waiter = tokio::spawn(async move {
    //            let buf = pool_clone.get_buffer().await;
    //            buf.buffer().capacity() + i // Return something identifiable
    //        });
    //        waiters.push(waiter);
    //    }

    //    // Give waiters time to start
    //    tokio::time::sleep(Duration::from_millis(10)).await;

    //    // Return the buffer - should wake up one waiter
    //    pool.return_buffer(buf);

    //    // Cancel remaining waiters
    //    for waiter in waiters.into_iter().skip(1) {
    //        waiter.abort();
    //    }

    //    assert_eq!(pool.available_buffers(), 1);
    //}

    // =============================================================================
    // 9. Advanced Usage Patterns
    // =============================================================================

    #[test]
    fn test_buffer_as_circular_buffer_simulation() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);

        // Simulate circular buffer behavior by managing manually
        let mut write_pos = 0;
        let capacity = buffer.capacity();

        // Write data in circular fashion
        for i in 0..20 {
            let (_, raw_data) = buffer.raw_split_mut();
            raw_data[write_pos % capacity] = Sample(i);
            write_pos += 1;

            // Update length to reflect actual used space
            let current_len = std::cmp::min(write_pos, capacity);
            buffer.header_mut().update_length(current_len);
        }

        // Should be full with the last 5 values
        assert!(buffer.is_full());
        let data = buffer.data();

        // Values should be 15, 16, 17, 18, 19 (in some order based on write position)
        let mut found_values: Vec<u32> = data.iter().map(|s| s.0).collect();
        found_values.sort();
        assert_eq!(found_values, vec![15, 16, 17, 18, 19]);
    }

    #[test]
    fn test_manual_header_manipulation() {
        let mut buffer = Buffer::<Sample>::with_capacity(5);

        // Manually fill some data
        let (header, data) = buffer.raw_split_mut();
        data[0] = Sample(100);
        data[1] = Sample(200);
        data[2] = Sample(300);

        // Update header manually
        header.update_length(3);

        // Verify buffer reflects the manual changes
        assert_eq!(buffer.len(), 3);
        let buffer_data = buffer.data();
        assert_eq!(buffer_data[0], Sample(100));
        assert_eq!(buffer_data[1], Sample(200));
        assert_eq!(buffer_data[2], Sample(300));
    }

    #[tokio::test]
    async fn test_buffer_sharing_between_tasks() {
        let pool = Arc::new(BufferPool::<Sample>::with_capacity(1, 10));

        // Producer task
        let pool_producer = Arc::clone(&pool);
        let producer = tokio::spawn(async move {
            let mut buf = pool_producer.get_buffer().await;

            // Fill buffer with data
            for i in 0..5 {
                buf.buffer_mut().push(Sample(i * 10));
            }

            pool_producer.return_buffer(buf);
        });

        // Consumer task
        let pool_consumer = Arc::clone(&pool);
        let consumer = tokio::spawn(async move {
            // Wait a bit for producer to finish
            tokio::time::sleep(Duration::from_millis(10)).await;

            let buf = pool_consumer.get_buffer().await;
            let sum: u32 = buf.buffer().data().iter().map(|s| s.0).sum();
            pool_consumer.return_buffer(buf);
            sum
        });

        let (_, sum) = tokio::join!(producer, consumer);

        // Should be 0 because buffer is cleared on return
        // This test demonstrates the current behavior
        assert_eq!(sum.unwrap(), 0);
    }
}
