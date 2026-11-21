//! Port of
//! `lucene/core/src/test/org/apache/lucene/util/TestRecyclingByteBlockAllocator.java`.
//!
//! Java's `HashSet<byte[]>` hashes blocks by identity; the port keys them by
//! their heap address and keeps them in insertion order. Java's
//! `toArray` order (identity-hash order) is not reproducible in any JVM run
//! either, so only the selection of `[begin, end)` differs from a given Java
//! run, never a draw.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::int_block_pool::ByteBlockAllocator;
use rucene::util::int_block_pool::RecyclingByteBlockAllocator;
use rucene::util::Counter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestRecyclingByteBlockAllocator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestRecyclingByteBlockAllocator",
        vec![
            test("testAllocate", test_allocate),
            test("testAllocateAndRecycle", test_allocate_and_recycle),
            test("testAllocateAndFree", test_allocate_and_free),
        ],
    )
}

fn new_allocator() -> RecyclingByteBlockAllocator {
    RecyclingByteBlockAllocator::with_counter(
        random().next_int_bound(97) as usize,
        Arc::new(Counter::new_counter()),
    )
}

/// The identity of a block (`byte[]` reference): its heap address.
fn id(block: &[u8]) -> usize {
    block.as_ptr() as usize
}

/// `HashSet<byte[]>` with identity semantics, in insertion order.
#[derive(Default)]
struct IdentitySet {
    ids: HashSet<usize>,
    blocks: Vec<Vec<u8>>,
}

impl IdentitySet {
    fn add(&mut self, block: Vec<u8>) -> bool {
        if self.ids.insert(id(&block)) {
            self.blocks.push(block);
            true
        } else {
            false
        }
    }

    fn size(&self) -> usize {
        self.ids.len()
    }

    /// `toArray(byte[][]::new)`: the blocks move into the array; the set
    /// keeps their identities.
    fn take_array(&mut self) -> Vec<Option<Vec<u8>>> {
        self.blocks.drain(..).map(Some).collect()
    }

    /// Puts back the blocks of `array` still present (those not recycled).
    fn restore(&mut self, array: Vec<Option<Vec<u8>>>) {
        for b in array.into_iter().flatten() {
            self.blocks.push(b);
        }
    }

    fn remove(&mut self, block_id: usize) -> bool {
        self.ids.remove(&block_id)
    }
}

fn test_allocate() -> Result<()> {
    let mut allocator = new_allocator();
    let mut set: HashSet<usize> = HashSet::new();
    let mut kept: Vec<Vec<u8>> = Vec::new();
    let block = allocator.get_byte_block();
    set.insert(id(&block));
    let size = block.len();
    kept.push(block);

    let num = LuceneTestCase::at_least(97);
    for i in 0..num {
        let block = allocator.get_byte_block();
        assert_equals(&size, &block.len());
        Assert::assert_true_msg("block is returned twice", set.insert(id(&block)));
        kept.push(block);
        assert_equals(&((size * (i as usize + 2)) as i64), &allocator.bytes_used()); // zero based + 1
        assert_equals(&0, &allocator.num_buffered_blocks());
    }
    Ok(())
}

fn test_allocate_and_recycle() -> Result<()> {
    let mut allocator = new_allocator();
    let mut allocated = IdentitySet::default();

    let block = allocator.get_byte_block();
    let size = block.len();
    allocated.add(block);

    let num_iters = LuceneTestCase::at_least(97);
    for _ in 0..num_iters {
        let num = 1 + random().next_int_bound(39);
        for _ in 0..num {
            let block = allocator.get_byte_block();
            assert_equals(&size, &block.len());
            Assert::assert_true_msg("block is returned twice", allocated.add(block));
            assert_equals(
                &((size * (allocated.size() + allocator.num_buffered_blocks())) as i64),
                &allocator.bytes_used(),
            );
        }
        let mut array = allocated.take_array();
        let begin = random().next_int_bound(array.len() as i32) as usize;
        let end = begin + random().next_int_bound((array.len() - begin) as i32) as usize;
        let mut selected: Vec<usize> = Vec::new();
        for j in begin..end {
            selected.push(id(array[j].as_ref().expect("allocated block")));
        }
        allocator.recycle_byte_blocks(&mut array, begin, end);
        for j in begin..end {
            Assert::assert_true(array[j].is_none());
            let b = selected.remove(0);
            Assert::assert_true(allocated.remove(b));
        }
        allocated.restore(array);
    }
    Ok(())
}

fn test_allocate_and_free() -> Result<()> {
    let mut allocator = new_allocator();
    let mut allocated = IdentitySet::default();
    let mut free_but_allocated = 0i32;
    let block = allocator.get_byte_block();
    let size = block.len();
    allocated.add(block);

    let num_iters = LuceneTestCase::at_least(97);
    for _ in 0..num_iters {
        let num = 1 + random().next_int_bound(39);
        for _ in 0..num {
            let block = allocator.get_byte_block();
            free_but_allocated = 0.max(free_but_allocated - 1);
            assert_equals(&size, &block.len());
            Assert::assert_true_msg("block is returned twice", allocated.add(block));
            assert_equals(
                &((size * (allocated.size() + allocator.num_buffered_blocks())) as i64),
                &allocator.bytes_used(),
            );
        }

        let mut array = allocated.take_array();
        let begin = random().next_int_bound(array.len() as i32) as usize;
        let end = begin + random().next_int_bound((array.len() - begin) as i32) as usize;
        for j in begin..end {
            let b = id(array[j].as_ref().expect("allocated block"));
            Assert::assert_true(allocated.remove(b));
        }
        allocator.recycle_byte_blocks(&mut array, begin, end);
        for j in begin..end {
            Assert::assert_true(array[j].is_none());
        }
        allocated.restore(array);
        // randomly free blocks
        let num_free_blocks = allocator.num_buffered_blocks();
        let free_blocks = allocator.free_blocks(
            random().next_int_bound(7 + allocator.max_buffered_blocks() as i32) as usize,
        );
        assert_equals(
            &allocator.num_buffered_blocks(),
            &(num_free_blocks - free_blocks),
        );
    }
    let _ = free_but_allocated;
    Ok(())
}
