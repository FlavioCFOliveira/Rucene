//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIntBlockPool.java`.
//!
//! tests basic {@link IntBlockPool} functionality
//!
//! * `pool.buffer` (the head buffer) is `buffer_mut(buffer_upto)`;
//!   `pool.buffers[i]` is `buffer(i)`; `pool.intUpto` and `pool.intOffset`
//!   are their accessors.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::int_block_pool::{
    DirectIntAllocator, IntBlockAllocator, IntBlockPool, INT_BLOCK_SIZE,
};
use rucene_test_framework::util::java_exceptions::is_instance;

use crate::core_support::{assert_equals, class, random, test};

/// `TestIntBlockPool`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIntBlockPool",
        vec![
            test("testWriteReadReset", test_write_read_reset),
            test("testTooManyAllocs", test_too_many_allocs),
        ],
    )
}

/// Port of `TestIntBlockPool.testWriteReadReset`.
fn test_write_read_reset() -> Result<()> {
    let mut pool = IntBlockPool::with_allocator(Box::new(DirectIntAllocator::new()));
    pool.next_buffer()?;

    // Write <count> consecutive ints to the buffer, possibly allocating a new buffer
    let mut count = random().next_int_bound(2 * INT_BLOCK_SIZE as i32) as usize;
    for i in 0..count {
        if pool.int_upto() == INT_BLOCK_SIZE {
            pool.next_buffer()?;
        }
        let upto = pool.int_upto();
        let head = pool.buffer_upto() as usize;
        pool.buffer_mut(head)[upto] = i as i32;
        pool.set_int_upto(upto + 1);
    }

    // Check that all the ints are present in th buffer pool
    for i in 0..count {
        assert_equals(
            &(i as i32),
            &pool.buffer(i / INT_BLOCK_SIZE)[i % INT_BLOCK_SIZE],
        );
    }

    // Reset without filling with zeros and check that the first buffer still has the ints
    count = count.min(INT_BLOCK_SIZE);
    pool.reset(false, true);
    for i in 0..count {
        assert_equals(&(i as i32), &pool.buffer(0)[i]);
    }

    // Reset and fill with zeros, then check there is no data left
    pool.set_int_upto(count);
    pool.reset(true, true);
    for i in 0..count {
        assert_equals(&0, &pool.buffer(0)[i]);
    }
    Ok(())
}

/// The anonymous `IntBlockPool.Allocator(0)` of `testTooManyAllocs`: a mock
/// allocator that doesn't waste memory.
struct EmptyAllocator;

impl IntBlockAllocator for EmptyAllocator {
    fn block_size(&self) -> usize {
        0
    }

    fn recycle_int_blocks(&mut self, _blocks: &mut [Option<Vec<i32>>], _start: usize, _end: usize) {
    }

    fn get_int_block(&mut self) -> Vec<i32> {
        Vec::new()
    }
}

/// Port of `TestIntBlockPool.testTooManyAllocs`.
fn test_too_many_allocs() -> Result<()> {
    // Use a mock allocator that doesn't waste memory
    let mut pool = IntBlockPool::with_allocator(Box::new(EmptyAllocator));
    pool.next_buffer()?;

    let mut throws_exception = false;
    for _ in 0..(i32::MAX / INT_BLOCK_SIZE as i32 + 1) {
        match pool.next_buffer() {
            Ok(()) => {}
            Err(e) if is_instance(&e, "java.lang.ArithmeticException") => {
                // The offset overflows on the last attempt to call nextBuffer()
                throws_exception = true;
                break;
            }
            Err(e) => return Err(e),
        }
    }
    Assert::assert_true(throws_exception);
    Assert::assert_true(pool.int_offset().wrapping_add(INT_BLOCK_SIZE as i32) < pool.int_offset());
    Ok(())
}
