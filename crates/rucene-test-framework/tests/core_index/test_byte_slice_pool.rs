//! Port of `lucene/core/src/test/org/apache/lucene/index/TestByteSlicePool.java`.
//!
//! * Rucene's `ByteSlicePool` functions take the pool; Java's `byte[]` block
//!   references (`blockPool.buffer`, `getBuffer(i)`) are block indexes
//!   (`buffer_upto()`, `i`).
//! * `allocKnownSizeSlice` returns `(offset, length)`, the pair Java packs
//!   into `offset << 8 | length`.
//! * Rucene's `RecyclingByteBlockAllocator` implements the `IntBlockPool`
//!   allocator trait rather than `ByteBlockPool`'s; [`RecyclingAllocator`]
//!   hands its blocks to the pool, as Java's `RecyclingByteBlockAllocator`
//!   (a `ByteBlockPool.Allocator`) does.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::freq_prox_terms_writer::{ByteSlicePool, FIRST_LEVEL_SIZE, LEVEL_SIZE_ARRAY};
use rucene::util::byte_block_pool::{Allocator, DirectAllocator, DirectTrackingAllocator};
use rucene::util::concurrent::Counter;
use rucene::util::{
    ByteBlockAllocator, ByteBlockPool, RecyclingByteBlockAllocator, BYTE_BLOCK_SIZE,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::TestUtil;

use crate::core_support::{
    assert_array_equals, assert_equals, assert_not_equals, class, random, test, IAE,
};

/// `TestByteSlicePool`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestByteSlicePool",
        vec![
            test("testAllocKnownSizeSlice", test_alloc_known_size_slice),
            test("testAllocLargeSlice", test_alloc_large_slice),
            test(
                "testRandomInterleavedSlices",
                test_random_interleaved_slices,
            ),
        ],
    )
}

/// `new RecyclingByteBlockAllocator()` as a `ByteBlockPool.Allocator`.
#[derive(Debug)]
struct RecyclingAllocator(Mutex<RecyclingByteBlockAllocator>);

impl Allocator for RecyclingAllocator {
    fn block_size(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .block_size()
    }

    fn recycle_byte_blocks(&self, blocks: &mut [Vec<u8>], start: usize, end: usize) {
        let mut slots: Vec<Option<Vec<u8>>> =
            blocks.iter_mut().map(|b| Some(std::mem::take(b))).collect();
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recycle_byte_blocks(&mut slots, start, end);
        for (block, slot) in blocks.iter_mut().zip(slots) {
            if let Some(slot) = slot {
                *block = slot;
            }
        }
    }

    fn get_byte_block(&self) -> Vec<u8> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_byte_block()
    }
}

/// Port of `TestByteSlicePool.testAllocKnownSizeSlice`.
fn test_alloc_known_size_slice() -> Result<()> {
    let bytes_used = Arc::new(Counter::new_counter());
    let mut block_pool =
        ByteBlockPool::with_allocator(Box::new(DirectTrackingAllocator::new(bytes_used)));
    block_pool.next_buffer()?;

    for _ in 0..100 {
        let size = if random().next_boolean() {
            TestUtil::next_int(&*random(), 100, 1000)
        } else {
            TestUtil::next_int(&*random(), 50000, 100000)
        } as usize;
        let mut random_data = vec![0u8; size];
        random().next_bytes(&mut random_data);

        let mut upto = ByteSlicePool::new_slice(&mut block_pool, FIRST_LEVEL_SIZE)?;

        let mut offset = 0;
        while offset < size {
            let buffer = block_pool.buffer_upto() as usize;
            if (block_pool.buffer(buffer)[upto] & 16) == 0 {
                block_pool.buffer_mut(buffer)[upto] = random_data[offset];
                upto += 1;
                offset += 1;
            } else {
                let (slice_offset, slice_length) =
                    ByteSlicePool::alloc_known_size_slice(&mut block_pool, buffer, upto)?;
                upto = slice_offset;
                let buffer = block_pool.buffer_upto() as usize;

                assert_not_equals(&0, &block_pool.buffer(buffer)[upto + slice_length - 1]);
                assert_equals(&0, &block_pool.buffer(buffer)[upto]);

                let write_length = (slice_length - 1).min(size - offset);
                block_pool.buffer_mut(buffer)[upto..upto + write_length]
                    .copy_from_slice(&random_data[offset..offset + write_length]);
                offset += write_length;
                upto += write_length;
            }
        }
    }
    Ok(())
}

/// Port of `TestByteSlicePool.testAllocLargeSlice`.
fn test_alloc_large_slice() -> Result<()> {
    let mut block_pool = ByteBlockPool::with_allocator(Box::new(DirectAllocator::new()));
    assert_equals(
        &0,
        &ByteSlicePool::new_slice(&mut block_pool, BYTE_BLOCK_SIZE)?,
    );
    assert_array_equals(
        block_pool.buffer(block_pool.buffer_upto() as usize),
        block_pool.buffer(0),
    );
    block_pool.next_buffer()?;
    expect_throws(IAE, || {
        ByteSlicePool::new_slice(&mut block_pool, BYTE_BLOCK_SIZE + 1)
    });
    Ok(())
}

/// `static class SliceWriter`: create a random byte array and write it to a
/// `ByteSlicePool` one slice at a time.
struct SliceWriter {
    has_started: bool,
    size: usize,
    random_data: Vec<u8>,
    data_offset: usize,
    slice: usize,
    slice_length: usize,
    slice_offset: usize,
    first_slice_offset: usize,
    first_slice: usize,
}

impl SliceWriter {
    fn new() -> Self {
        let size = if random().next_boolean() {
            // size < ByteBlockPool.BYTE_BLOCK_SIZE
            TestUtil::next_int(&*random(), 100, 1000)
        } else {
            // size > ByteBlockPool.BYTE_BLOCK_SIZE
            TestUtil::next_int(&*random(), 50000, 100000)
        } as usize;
        let mut random_data = vec![0u8; size];
        random().next_bytes(&mut random_data);
        SliceWriter {
            has_started: false,
            size,
            random_data,
            data_offset: 0,
            slice: 0,
            slice_length: 0,
            slice_offset: 0,
            first_slice_offset: 0,
            first_slice: 0,
        }
    }

    /// Write the next slice of data.
    ///
    /// Returns true if we wrote a slice and false if we're out of data to
    /// write.
    fn write_slice(&mut self, block_pool: &mut ByteBlockPool) -> Result<bool> {
        // The first slice is special
        if !self.has_started {
            self.data_offset = 0; // Offset into the data buffer
            self.slice_length = FIRST_LEVEL_SIZE;
            self.slice_offset = ByteSlicePool::new_slice(block_pool, self.slice_length)?;
            self.first_slice_offset = self.slice_offset; // We will need this later
            self.first_slice = block_pool.buffer_upto() as usize; // We will need this later
            self.slice = self.first_slice;
            let write_length = self.size.min(self.slice_length - 1);
            let buffer = block_pool.buffer_upto() as usize;
            block_pool.buffer_mut(buffer)[self.slice_offset..self.slice_offset + write_length]
                .copy_from_slice(
                    &self.random_data[self.data_offset..self.data_offset + write_length],
                );
            self.data_offset += write_length;
            self.has_started = true;
            return Ok(true);
        }

        // Have we written everything?
        if self.data_offset == self.size {
            return Ok(false);
        }

        // No, write more
        let (offset, length) = ByteSlicePool::alloc_known_size_slice(
            block_pool,
            self.slice,
            self.slice_offset + self.slice_length - 1,
        )?;
        self.slice = block_pool.buffer_upto() as usize;
        self.slice_length = length;
        self.slice_offset = offset;
        let write_length = (self.size - self.data_offset).min(self.slice_length - 1);
        block_pool.buffer_mut(self.slice)[self.slice_offset..self.slice_offset + write_length]
            .copy_from_slice(&self.random_data[self.data_offset..self.data_offset + write_length]);
        self.data_offset += write_length;
        Ok(true)
    }
}

/// `static class SliceReader`: read a sequence of slices into a byte array.
struct SliceReader {
    has_started: bool,
    size: usize,
    read_data: Vec<u8>,
    data_offset: usize,
    slice_length: usize,
    slice_offset: usize,
    slice: usize,
    slice_size_idx: usize,
}

impl SliceReader {
    fn new(size: usize, first_slice_offset: usize, first_slice: usize) -> Self {
        SliceReader {
            has_started: false,
            size,
            read_data: vec![0u8; size],
            data_offset: 0,
            slice_length: 0,
            slice_offset: first_slice_offset,
            slice: first_slice,
            slice_size_idx: 0,
        }
    }

    /// Read the next slice of data.
    ///
    /// Returns true if we read a slice and false if we'd already read the
    /// entire sequence of slices.
    fn read_slice(&mut self, block_pool: &ByteBlockPool) -> bool {
        // The first slice is special
        if !self.has_started {
            self.data_offset = 0;
            // Index into LEVEL_SIZE_ARRAY, allowing us to find the size of the current slice
            self.slice_size_idx = 0;
            // 4 bytes are for the offset to the next slice, we can't use them for data
            self.slice_length = LEVEL_SIZE_ARRAY[self.slice_size_idx] - 4;
            self.copy_slice(block_pool);
            self.slice_size_idx = (self.slice_size_idx + 1).min(LEVEL_SIZE_ARRAY.len() - 1);
            self.has_started = true;
            return true;
        }

        // Have we read everything?
        if self.data_offset == self.size {
            return false;
        }

        // No, read more
        let at = self.slice_offset + self.slice_length;
        let global_slice_offset = i32::from_le_bytes(
            block_pool.buffer(self.slice)[at..at + 4]
                .try_into()
                .expect("four bytes"),
        ) as usize;
        self.slice = global_slice_offset / BYTE_BLOCK_SIZE;
        self.slice_offset = global_slice_offset % BYTE_BLOCK_SIZE;
        self.slice_length = LEVEL_SIZE_ARRAY[self.slice_size_idx] - 4;
        self.copy_slice(block_pool);
        self.slice_size_idx = (self.slice_size_idx + 1).min(LEVEL_SIZE_ARRAY.len() - 1);
        true
    }

    /// The read the two branches of `readSlice` share.
    fn copy_slice(&mut self, block_pool: &ByteBlockPool) {
        let read_length = if self.data_offset + self.slice_length + 3 >= self.size {
            // We are reading the last slice, there is no more offset, just a byte for the level
            self.size - self.data_offset
        } else {
            self.slice_length
        };
        self.read_data[self.data_offset..self.data_offset + read_length].copy_from_slice(
            &block_pool.buffer(self.slice)[self.slice_offset..self.slice_offset + read_length],
        );
        self.data_offset += read_length;
    }
}

/// Port of `TestByteSlicePool.testRandomInterleavedSlices`: run multiple
/// slice writers, creating interleaved slices. Read the slices afterwards and
/// check that we read back the same data we wrote.
fn test_random_interleaved_slices() -> Result<()> {
    let mut block_pool = ByteBlockPool::with_allocator(Box::new(RecyclingAllocator(Mutex::new(
        RecyclingByteBlockAllocator::new(),
    ))));

    let n_iterations = TestUtil::next_int(&*random(), 1, 3); // 1-3 iterations with buffer resets in between
    for _ in 0..n_iterations {
        let n = TestUtil::next_int(&*random(), 2, 3) as usize; // 2 or 3 writers and readers
        let mut slice_writers: Vec<SliceWriter> = Vec::with_capacity(n);

        // Init slice writers
        for _ in 0..n {
            slice_writers.push(SliceWriter::new());
        }

        // Write slices
        loop {
            let i = random().next_int_bound(n as i32) as usize;
            let succeeded = slice_writers[i].write_slice(&mut block_pool)?;
            if !succeeded {
                for j in 0..n {
                    while slice_writers[j].write_slice(&mut block_pool)? {}
                }
                break;
            }
        }

        // Init slice readers
        let mut slice_readers: Vec<SliceReader> = slice_writers
            .iter()
            .map(|w| SliceReader::new(w.size, w.first_slice_offset, w.first_slice))
            .collect();

        // Read slices
        loop {
            let i = random().next_int_bound(n as i32) as usize;
            let succeeded = slice_readers[i].read_slice(&block_pool);
            if !succeeded {
                for j in 0..n {
                    while slice_readers[j].read_slice(&block_pool) {}
                }
                break;
            }
        }

        // Compare written data with read data
        for i in 0..n {
            assert_array_equals(&slice_writers[i].random_data, &slice_readers[i].read_data);
        }

        // We don't rely on the buffers being filled with zeros because the SliceWriter keeps the
        // slice length as state, but ByteSlicePool.allocKnownSizeSlice asserts on zeros in the
        // buffer.
        block_pool.reset(true, random().next_boolean());
    }
    Ok(())
}
