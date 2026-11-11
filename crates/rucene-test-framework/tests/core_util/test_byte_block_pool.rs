//! Port of `lucene/core/src/test/org/apache/lucene/util/TestByteBlockPool.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomBytes;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::byte_block_pool::{Allocator, DirectAllocator, DirectTrackingAllocator};
use rucene::util::concurrent::Counter;
use rucene::util::{ByteBlockPool, BytesRef, BytesRefBuilder, BYTE_BLOCK_SIZE};
use rucene_test_framework::util::java_exceptions::is_instance;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};

/// `TestByteBlockPool`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestByteBlockPool",
        vec![
            test("testAppendFromOtherPool", test_append_from_other_pool),
            test("testReadAndWrite", test_read_and_write),
            test("testLargeRandomBlocks", test_large_random_blocks),
            test("testTooManyAllocs", test_too_many_allocs),
        ],
    )
}

fn test_append_from_other_pool() -> Result<()> {
    let random = random();

    let mut pool = ByteBlockPool::with_allocator(Box::new(DirectAllocator::new()));
    let num_bytes = LuceneTestCase::at_least(2 << 16);
    let bytes = RandomBytes::random_bytes_of_length(&*random, num_bytes);
    pool.append(&bytes)?;

    let mut another_pool = ByteBlockPool::with_allocator(Box::new(DirectAllocator::new()));
    let existing_bytes = vec![0u8; LuceneTestCase::at_least(500) as usize];
    another_pool.append(&existing_bytes)?;

    // now slice and append to another pool
    let offset = TestUtil::next_int(&*random, 1, 2 << 15);
    let mut length = bytes.len() as i32 - offset;
    if random.next_boolean() {
        length = TestUtil::next_int(&*random, 1, length);
    }
    another_pool.append_pool(&pool, offset as i64, length as usize)?;

    assert_equals(
        &(existing_bytes.len() as i64 + length as i64),
        &another_pool.position(),
    );

    let mut results = vec![0u8; length as usize];
    let results_len = results.len();
    another_pool.read_bytes(existing_bytes.len() as i64, &mut results, 0, results_len);
    for i in 0..length as usize {
        assert_equals_msg(
            Some(&format!("byte @ index={i}")),
            &bytes[offset as usize + i],
            &results[i],
        );
    }
    Ok(())
}

fn test_read_and_write() -> Result<()> {
    let bytes_used = Arc::new(Counter::new_counter());
    let mut pool = ByteBlockPool::with_allocator(Box::new(DirectTrackingAllocator::new(
        Arc::clone(&bytes_used),
    )));
    pool.next_buffer()?;
    let reuse_first = random().next_boolean();
    for _ in 0..2 {
        let mut list: Vec<BytesRef> = Vec::new();
        let max_length = LuceneTestCase::at_least(500);
        let num_values = LuceneTestCase::at_least(100);
        let mut r#ref = BytesRefBuilder::new();
        for _ in 0..num_values {
            let value = TestUtil::random_realistic_unicode_string_max(&*random(), max_length);
            list.push(BytesRef::from_char_sequence(&value));
            r#ref.copy_chars(&value);
            pool.append(r#ref.get().slice())?;
        }
        // verify
        let mut position: i64 = 0;
        let mut builder = BytesRefBuilder::new();
        for expected in &list {
            r#ref.grow(expected.length);
            r#ref.set_length(expected.length);
            match random().next_int_bound(2) {
                0 => {
                    // copy bytes
                    let len = r#ref.length();
                    pool.read_bytes(position, r#ref.bytes_mut(), 0, len);
                }
                1 => {
                    let len = r#ref.length();
                    let scratch = pool.set_bytes_ref(&mut builder, position, len).to_vec();
                    r#ref.bytes_mut()[..len].copy_from_slice(&scratch[..len]);
                }
                _ => Assert::fail_without_message(),
            }
            assert_equals(expected, &r#ref.get());
            position += r#ref.length() as i64;
        }
        pool.reset(random().next_boolean(), reuse_first);
        if reuse_first {
            assert_equals(&(BYTE_BLOCK_SIZE as i64), &bytes_used.get());
        } else {
            assert_equals(&0, &bytes_used.get());
            pool.next_buffer()?; // prepare for next iter
        }
    }
    Ok(())
}

fn test_large_random_blocks() -> Result<()> {
    let bytes_used = Arc::new(Counter::new_counter());
    let mut pool =
        ByteBlockPool::with_allocator(Box::new(DirectTrackingAllocator::new(bytes_used)));
    pool.next_buffer()?;

    let mut total_bytes: i64 = 0;
    let mut items: Vec<Vec<u8>> = Vec::new();
    for _ in 0..100 {
        let size = if random().next_boolean() {
            TestUtil::next_int(&*random(), 100, 1000)
        } else {
            TestUtil::next_int(&*random(), 50000, 100000)
        };
        let mut bytes = vec![0u8; size as usize];
        random().next_bytes(&mut bytes);
        pool.append(BytesRef::new(bytes.clone()).slice())?;
        items.push(bytes);
        total_bytes += size as i64;

        // make sure we report the correct position
        assert_equals(&total_bytes, &pool.position());
    }

    let mut position: i64 = 0;
    for expected in &items {
        let mut actual = vec![0u8; expected.len()];
        let len = actual.len();
        pool.read_bytes(position, &mut actual, 0, len);
        Assert::assert_true(*expected == actual);
        position += expected.len() as i64;
    }
    Ok(())
}

/// The anonymous `ByteBlockPool.Allocator(0)` of `testTooManyAllocs`: "a
/// mock allocator that doesn't waste memory", handing out one empty
/// `buffer`.
#[derive(Debug)]
struct EmptyBlockAllocator;

impl Allocator for EmptyBlockAllocator {
    fn block_size(&self) -> usize {
        0
    }

    fn recycle_byte_blocks(&self, _blocks: &mut [Vec<u8>], _start: usize, _end: usize) {}

    fn get_byte_block(&self) -> Vec<u8> {
        Vec::new()
    }
}

fn test_too_many_allocs() -> Result<()> {
    // Use a mock allocator that doesn't waste memory
    let mut pool = ByteBlockPool::with_allocator(Box::new(EmptyBlockAllocator));
    pool.next_buffer()?;

    let mut throws_exception = false;
    for _ in 0..(i32::MAX / BYTE_BLOCK_SIZE as i32 + 1) {
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
    Assert::assert_true(
        pool.byte_offset().wrapping_add(BYTE_BLOCK_SIZE as i32) < pool.byte_offset(),
    );
    Ok(())
}
