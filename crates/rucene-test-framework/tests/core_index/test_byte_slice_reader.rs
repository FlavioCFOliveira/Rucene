//! Port of `lucene/core/src/test/org/apache/lucene/index/TestByteSliceReader.java`.
//!
//! Rucene's `ByteSliceReader` keeps the pool out of the cursor; the reader
//! that reads like Java's (`DataInput`) is `PooledSliceReader`, the cursor
//! over the pool. Java's `byte[] buffer` reference is the pool's head block,
//! `buffer_upto()`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::error::Result;
use rucene::index::freq_prox_terms_writer::{
    ByteSlicePool, ByteSliceReader, PooledSliceReader, FIRST_LEVEL_SIZE,
};
use rucene::store::DataInput;
use rucene::util::byte_block_pool::DirectAllocator;
use rucene::util::ByteBlockPool;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test};
use crate::support::{npe, static_hook};

/// `TestByteSliceReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestByteSliceReader",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testReadByte", test_read_byte),
            test("testSkipBytes", test_skip_bytes),
        ],
    )
}

/// `RANDOM_DATA`, `BLOCK_POOL` and `BLOCK_POOL_END`.
struct Statics {
    random_data: Vec<u8>,
    block_pool: ByteBlockPool,
    block_pool_end: i32,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn statics() -> std::sync::MutexGuard<'static, Option<Statics>> {
    STATICS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `@BeforeClass public static void beforeClass()`.
fn before_class() -> Result<()> {
    let len = LuceneTestCase::at_least(100);
    let mut random_data = vec![0u8; len as usize];
    random().next_bytes(&mut random_data);

    let mut block_pool = ByteBlockPool::with_allocator(Box::new(DirectAllocator::new()));
    block_pool.next_buffer()?;
    let mut buffer = block_pool.buffer_upto() as usize;
    let mut upto = ByteSlicePool::new_slice(&mut block_pool, FIRST_LEVEL_SIZE)?;
    for &random_byte in &random_data {
        if (block_pool.buffer(buffer)[upto] & 16) != 0 {
            upto = ByteSlicePool::alloc_slice(&mut block_pool, buffer, upto)?;
            buffer = block_pool.buffer_upto() as usize;
        }
        block_pool.buffer_mut(buffer)[upto] = random_byte;
        upto += 1;
    }
    *statics() = Some(Statics {
        random_data,
        block_pool,
        block_pool_end: upto as i32,
    });
    Ok(())
}

/// `@AfterClass public static void afterClass()`.
fn after_class() -> Result<()> {
    *statics() = None;
    Ok(())
}

/// Port of `TestByteSliceReader.testReadByte`.
fn test_read_byte() -> Result<()> {
    let guard = statics();
    let s = guard.as_ref().unwrap_or_else(|| npe("BLOCK_POOL"));
    let mut slice_reader = ByteSliceReader::new();
    slice_reader.init(0, s.block_pool_end);
    let mut slice_reader = PooledSliceReader::new(slice_reader, &s.block_pool);
    for &expected in &s.random_data {
        assert_equals(&expected, &slice_reader.read_byte()?);
    }
    Ok(())
}

/// Port of `TestByteSliceReader.testSkipBytes`.
fn test_skip_bytes() -> Result<()> {
    let guard = statics();
    let s = guard.as_ref().unwrap_or_else(|| npe("BLOCK_POOL"));
    let random = random();
    let max_skip_to = s.random_data.len() as i32 - 1;
    let iterations = LuceneTestCase::at_least_with(&*random, 10);
    for _ in 0..iterations {
        let mut cursor = ByteSliceReader::new();
        cursor.init(0, s.block_pool_end);
        let mut slice_reader = PooledSliceReader::new(cursor, &s.block_pool);
        // skip random chunks of bytes until exhausted
        let mut curr = 0;
        while curr < max_skip_to {
            let skip_to = TestUtil::next_int(&*random, curr, max_skip_to);
            let step = skip_to - curr;
            slice_reader.skip_bytes(step as i64)?;
            assert_equals(&s.random_data[skip_to as usize], &slice_reader.read_byte()?);
            curr = skip_to + 1; // +1 for read byte
        }
    }
    Ok(())
}
