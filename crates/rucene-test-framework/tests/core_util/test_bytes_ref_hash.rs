//! Port of `lucene/core/src/test/org/apache/lucene/util/TestBytesRefHash.java`.
//!
//! Rucene's `BytesRefHash` takes its `ByteBlockPool` as a parameter of each
//! call rather than holding it, and chooses at construction whether it is
//! keyed by the bytes (`add`) or by pool offsets (`addByPoolOffset`) — see
//! `rucene::util::bytes_ref_hash`. The instance fields `hash` and `pool` are
//! therefore passed together, and `offsetHash` is built keyed by pool
//! offsets.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::byte_block_pool::Allocator;
use rucene::util::bytes_ref_hash::DirectBytesStartArray;
use rucene::util::{
    ByteBlockAllocator, ByteBlockPool, BytesRef, BytesRefBuilder, BytesRefHash,
    RecyclingByteBlockAllocator, BYTE_BLOCK_SIZE,
};
use rucene_test_framework::jdk::bit_set::JavaBitSet;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_null, random};

/// `org.apache.lucene.util.BytesRefHash.MaxBytesLengthExceededException`.
const MAX_BYTES_LENGTH_EXCEEDED: &str =
    "org.apache.lucene.util.BytesRefHash$MaxBytesLengthExceededException";

/// A `RecyclingByteBlockAllocator` as the `ByteBlockPool.Allocator` it
/// extends in Java.
///
/// **Divergence from Lucene 10.5.0 (pre-existing, bridged here).** Rucene
/// declares `ByteBlockPool.Allocator` twice: `byte_block_pool::Allocator`,
/// which `ByteBlockPool` takes (`&self`, `Send + Sync`), and
/// `int_block_pool::ByteBlockAllocator`, which `RecyclingByteBlockAllocator`
/// implements (`&mut self`). This adapter forwards the first to the second
/// under a lock.
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
            *block = slot.unwrap_or_default();
        }
    }

    fn get_byte_block(&self) -> Vec<u8> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_byte_block()
    }
}

/// The instance of `TestBytesRefHash`: its fields `hash` and `pool`.
#[derive(Default)]
pub struct TestBytesRefHash {
    hash: Option<BytesRefHash>,
    pool: Option<ByteBlockPool>,
}

/// An instance method of `TestBytesRefHash`.
fn method(name: &str, body: fn(&mut TestBytesRefHash) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body::<TestBytesRefHash>(move |t| {
            if let Err(e) = body(t) {
                rethrow_java(e)
            }
        })
}

/// `TestBytesRefHash`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        method("setUp", |t| {
            t.set_up();
            Ok(())
        }),
        method("testSize", TestBytesRefHash::test_size),
        method("testGet", TestBytesRefHash::test_get),
        method("testCompact", TestBytesRefHash::test_compact),
        method("testSort", TestBytesRefHash::test_sort),
        method("testAdd", TestBytesRefHash::test_add),
        method("testFind", TestBytesRefHash::test_find),
        method(
            "testConcurrentAccessToBytesRefHash",
            TestBytesRefHash::test_concurrent_access_to_bytes_ref_hash,
        ),
        method("testLargeValue", TestBytesRefHash::test_large_value),
        method(
            "testAddByPoolOffset",
            TestBytesRefHash::test_add_by_pool_offset,
        ),
    ];
    let mut builder = Class::builder("org.apache.lucene.util.TestBytesRefHash")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestBytesRefHash::default));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// A random string of `TestUtil.randomRealisticUnicodeString(random(), 1000)`
/// that is not empty.
fn non_empty_string() -> String {
    loop {
        let str = TestUtil::random_realistic_unicode_string_max(&*random(), 1000);
        if !str.is_empty() {
            return str;
        }
    }
}

/// `hash.get(id, scratch)`.
fn get(hash: &BytesRefHash, pool: &ByteBlockPool, id: i32) -> BytesRef {
    BytesRef::new(hash.get(pool, id).to_vec())
}

/// `hash.get(id, scratch).utf8ToString()`.
fn get_string(hash: &BytesRefHash, pool: &ByteBlockPool, id: i32) -> String {
    get(hash, pool, id)
        .utf8_to_string()
        .unwrap_or_else(|e| rethrow_java(e))
}

impl TestBytesRefHash {
    /// `setUp()`.
    fn set_up(&mut self) {
        LuceneTestCase::super_set_up();
        self.pool = Some(self.new_pool());
        self.hash = Some(Self::new_hash(false));
    }

    /// `newPool()`: Java returns the current `pool` when one exists and the
    /// coin says so.
    fn new_pool(&mut self) -> ByteBlockPool {
        if random().next_boolean() && self.pool.is_some() {
            self.pool.take().expect("pool")
        } else {
            ByteBlockPool::with_allocator(Box::new(RecyclingAllocator(Mutex::new(
                RecyclingByteBlockAllocator::with_max_buffered_blocks(
                    random().next_int_bound(25) as usize
                ),
            ))))
        }
    }

    /// `newHash(blockPool)`; `by_pool_offset` builds the table
    /// `addByPoolOffset` adds to.
    fn new_hash(by_pool_offset: bool) -> BytesRefHash {
        let init_size = 2usize << (1 + random().next_int_bound(5));
        if random().next_boolean() {
            if by_pool_offset {
                BytesRefHash::new_by_pool_offset(Arc::new(AtomicI64::new(0)))
            } else {
                BytesRefHash::new(Arc::new(AtomicI64::new(0)))
            }
        } else {
            let start_array = Box::new(DirectBytesStartArray::with_private_counter(init_size));
            if by_pool_offset {
                BytesRefHash::with_bytes_start_array_by_pool_offset(init_size, start_array)
            } else {
                BytesRefHash::with_bytes_start_array(init_size, start_array)
            }
        }
    }

    fn parts(&mut self) -> (&mut BytesRefHash, &mut ByteBlockPool) {
        (
            self.hash.as_mut().expect("hash"),
            self.pool.as_mut().expect("pool"),
        )
    }

    /// Test method for `BytesRefHash#size()`.
    fn test_size(&mut self) -> Result<()> {
        let (hash, pool) = self.parts();
        let mut r#ref = BytesRefBuilder::new();
        let num = LuceneTestCase::at_least(2);
        for _ in 0..num {
            let r#mod = 1 + random().next_int_bound(39);
            for i in 0..797 {
                let str = non_empty_string();
                r#ref.copy_chars(&str);
                let count = hash.size();
                let key = hash.add(pool, r#ref.get().slice())?;
                if key < 0 {
                    assert_equals(&hash.size(), &count);
                } else {
                    assert_equals(&hash.size(), &(count + 1));
                }
                if i % r#mod == 0 {
                    hash.clear();
                    assert_equals(&0, &hash.size());
                    hash.reinit();
                }
            }
        }
        Ok(())
    }

    /// Test method for `BytesRefHash#get(int, BytesRef)`.
    fn test_get(&mut self) -> Result<()> {
        let (hash, pool) = self.parts();
        let mut r#ref = BytesRefBuilder::new();
        let num = LuceneTestCase::at_least(2);
        for _ in 0..num {
            let mut strings: HashMap<String, i32> = HashMap::new();
            let mut unique_count = 0;
            for _ in 0..797 {
                let str = non_empty_string();
                r#ref.copy_chars(&str);
                let count = hash.size();
                let key = hash.add(pool, r#ref.get().slice())?;
                if key >= 0 {
                    assert_null(&strings.insert(str, key));
                    assert_equals(&unique_count, &key);
                    unique_count += 1;
                    assert_equals(&hash.size(), &(count + 1));
                } else {
                    Assert::assert_true((-key) - 1 < count);
                    assert_equals(&hash.size(), &count);
                }
            }
            for (key, value) in &strings {
                r#ref.copy_chars(key);
                assert_equals(&r#ref.get(), &get(hash, pool, *value));
            }
            hash.clear();
            assert_equals(&0, &hash.size());
            hash.reinit();
        }
        Ok(())
    }

    /// Test method for `BytesRefHash#compact()`.
    fn test_compact(&mut self) -> Result<()> {
        let (hash, pool) = self.parts();
        let mut r#ref = BytesRefBuilder::new();
        let num = LuceneTestCase::at_least(2);
        for _ in 0..num {
            let mut num_entries = 0;
            let size = 797;
            let mut bits = JavaBitSet::with_capacity(size);
            for _ in 0..size {
                let str = non_empty_string();
                r#ref.copy_chars(&str);
                let key = hash.add(pool, r#ref.get().slice())?;
                if key < 0 {
                    Assert::assert_true(bits.get(((-key) - 1) as usize));
                } else {
                    Assert::assert_false(bits.get(key as usize));
                    bits.set(key as usize);
                    num_entries += 1;
                }
            }
            assert_equals(&(hash.size() as usize), &bits.cardinality());
            assert_equals(&num_entries, &bits.cardinality());
            assert_equals(&(num_entries as i32), &hash.size());
            let compact: Vec<i32> = hash.compact().to_vec();
            Assert::assert_true(num_entries < compact.len());
            for i in 0..num_entries {
                bits.clear(compact[i] as usize);
            }
            assert_equals(&0, &bits.cardinality());
            hash.clear();
            assert_equals(&0, &hash.size());
            hash.reinit();
        }
        Ok(())
    }

    /// Test method for `BytesRefHash#sort()`.
    fn test_sort(&mut self) -> Result<()> {
        let (hash, pool) = self.parts();
        let mut r#ref = BytesRefBuilder::new();
        let num = LuceneTestCase::at_least(2);
        for _ in 0..num {
            // Sorts by unicode code point order (is there a simple way, e.g. a Collator?)
            // `new TreeSet<>(TestUtil.STRING_CODEPOINT_COMPARATOR)`: Rust orders
            // `String`s by their UTF-8 bytes, which is code point order.
            let mut strings: BTreeSet<String> = BTreeSet::new();
            for _ in 0..797 {
                let str = non_empty_string();
                r#ref.copy_chars(&str);
                hash.add(pool, r#ref.get().slice())?;
                strings.insert(str);
            }
            for _ in 0..3 {
                // Test duplicate sort on a BytesRefHash instance work well. This makes no sense but some
                // users need that.
                let sort: Vec<i32> = hash.sort(pool).to_vec();
                Assert::assert_true(strings.len() < sort.len());
                let mut i = 0;
                for string in &strings {
                    r#ref.copy_chars(string);
                    assert_equals(&r#ref.get(), &get(hash, pool, sort[i]));
                    i += 1;
                }
            }
            hash.clear();
            assert_equals(&0, &hash.size());
            hash.reinit();
        }
        Ok(())
    }

    /// Test method for `BytesRefHash#add(BytesRef)`.
    fn test_add(&mut self) -> Result<()> {
        let (hash, pool) = self.parts();
        let mut r#ref = BytesRefBuilder::new();
        let num = LuceneTestCase::at_least(2);
        for _ in 0..num {
            let mut strings: HashSet<String> = HashSet::new();
            let mut unique_count = 0;
            for _ in 0..797 {
                let str = non_empty_string();
                r#ref.copy_chars(&str);
                let count = hash.size();
                let key = hash.add(pool, r#ref.get().slice())?;

                if key >= 0 {
                    Assert::assert_true(strings.insert(str));
                    assert_equals(&unique_count, &key);
                    assert_equals(&hash.size(), &(count + 1));
                    unique_count += 1;
                } else {
                    Assert::assert_false(strings.insert(str.clone()));
                    Assert::assert_true((-key) - 1 < count);
                    assert_equals(&str, &get_string(hash, pool, (-key) - 1));
                    assert_equals(&count, &hash.size());
                }
            }

            assert_all_in(&strings, hash, pool)?;
            hash.clear();
            assert_equals(&0, &hash.size());
            hash.reinit();
        }
        Ok(())
    }

    fn test_find(&mut self) -> Result<()> {
        let (hash, pool) = self.parts();
        let mut r#ref = BytesRefBuilder::new();
        let num = LuceneTestCase::at_least(2);
        for _ in 0..num {
            let mut strings: HashSet<String> = HashSet::new();
            let mut unique_count = 0;
            for _ in 0..797 {
                let str = non_empty_string();
                r#ref.copy_chars(&str);
                let count = hash.size();
                let mut key = hash.find(pool, r#ref.get().slice()); // hash.add(ref);
                if key >= 0 {
                    // string found in hash
                    Assert::assert_false(strings.insert(str.clone()));
                    Assert::assert_true(key < count);
                    assert_equals(&str, &get_string(hash, pool, key));
                    assert_equals(&count, &hash.size());
                } else {
                    key = hash.add(pool, r#ref.get().slice())?;
                    Assert::assert_true(strings.insert(str));
                    assert_equals(&unique_count, &key);
                    assert_equals(&hash.size(), &(count + 1));
                    unique_count += 1;
                }
            }

            assert_all_in(&strings, hash, pool)?;
            hash.clear();
            assert_equals(&0, &hash.size());
            hash.reinit();
        }
        Ok(())
    }

    fn test_concurrent_access_to_bytes_ref_hash(&mut self) -> Result<()> {
        let num = LuceneTestCase::at_least(2);
        for _ in 0..num {
            let num_strings = 797;
            let mut strings: Vec<String> = Vec::with_capacity(num_strings);
            {
                let (hash, pool) = self.parts();
                for _ in 0..num_strings {
                    let str =
                        TestUtil::random_realistic_unicode_string_between(&*random(), 1, 1000);
                    hash.add(pool, LuceneTestCase::new_bytes_ref_str(&str).slice())?;
                    strings.push(str);
                    Assert::assert_true(true);
                }
            }
            let strings = Arc::new(strings);
            // The threads read `hash` and `pool` concurrently, as Java's do.
            let shared = Arc::new((
                self.hash.take().expect("hash"),
                self.pool.take().expect("pool"),
            ));
            let hash_size = shared.0.size();

            let not_found = Arc::new(AtomicI32::new(0));
            let not_equals = Arc::new(AtomicI32::new(0));
            let wrong_size = Arc::new(AtomicI32::new(0));
            let num_threads = LuceneTestCase::at_least(3);
            // `CountDownLatch(numThreads)` counted down and awaited by every
            // thread.
            let latch = Arc::new(Barrier::new(num_threads as usize));
            let mut threads: Vec<Thread> = Vec::with_capacity(num_threads as usize);
            for i in 0..num_threads {
                let loops = LuceneTestCase::at_least(100);
                let shared = Arc::clone(&shared);
                let strings = Arc::clone(&strings);
                let latch = Arc::clone(&latch);
                let not_found = Arc::clone(&not_found);
                let not_equals = Arc::clone(&not_equals);
                let wrong_size = Arc::clone(&wrong_size);
                threads.push(Thread::new(format!("t{i}"), move || {
                    let (hash, pool) = &*shared;
                    latch.wait();
                    for k in 0..loops as usize {
                        let find = LuceneTestCase::new_bytes_ref_str(&strings[k % strings.len()]);
                        let id = hash.find(pool, find.slice());
                        if id < 0 {
                            not_found.fetch_add(1, Ordering::SeqCst);
                        } else {
                            let get = hash.get(pool, id);
                            if get != find.slice() {
                                not_equals.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        if hash.size() != hash_size {
                            wrong_size.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }));
            }

            for t in &threads {
                t.start();
            }
            for t in &threads {
                let _ = t.join();
            }
            drop(threads);

            assert_equals(&0, &not_found.load(Ordering::SeqCst));
            assert_equals(&0, &not_equals.load(Ordering::SeqCst));
            assert_equals(&0, &wrong_size.load(Ordering::SeqCst));
            let (hash, pool) = Arc::try_unwrap(shared)
                .unwrap_or_else(|_| panic!("the reader threads were joined"));
            self.hash = Some(hash);
            self.pool = Some(pool);
            let (hash, _) = self.parts();
            hash.clear();
            assert_equals(&0, &hash.size());
            hash.reinit();
        }
        Ok(())
    }

    fn test_large_value(&mut self) -> Result<()> {
        let (hash, pool) = self.parts();
        let sizes: [usize; 2] = [
            random().next_int_bound(5) as usize,
            BYTE_BLOCK_SIZE - 33 + random().next_int_bound(31) as usize,
        ];
        let mut r#ref = BytesRef::default();
        for i in 0..sizes.len() {
            r#ref.bytes = vec![0u8; sizes[i]];
            r#ref.offset = 0;
            r#ref.length = sizes[i];
            assert_equals(&(i as i32), &hash.add(pool, r#ref.slice())?);
        }

        r#ref.bytes = vec![0u8; BYTE_BLOCK_SIZE - 1 + random().next_int_bound(37) as usize];
        r#ref.offset = 0;
        r#ref.length = r#ref.bytes.len();
        expect_throws(MAX_BYTES_LENGTH_EXCEEDED, || hash.add(pool, r#ref.slice()));
        Ok(())
    }

    /// Test method for `BytesRefHash#addByPoolOffset(int)`.
    fn test_add_by_pool_offset(&mut self) -> Result<()> {
        let mut r#ref = BytesRefBuilder::new();
        let mut offset_hash = Self::new_hash(true);
        let (hash, pool) = self.parts();
        let num = LuceneTestCase::at_least(2);
        for _ in 0..num {
            let mut strings: HashSet<String> = HashSet::new();
            let mut unique_count = 0;
            for _ in 0..797 {
                let str = non_empty_string();
                r#ref.copy_chars(&str);
                let count = hash.size();
                let key = hash.add(pool, r#ref.get().slice())?;

                if key >= 0 {
                    Assert::assert_true(strings.insert(str));
                    assert_equals(&unique_count, &key);
                    assert_equals(&hash.size(), &(count + 1));
                    let offset_key = offset_hash.add_by_pool_offset(hash.byte_start(key));
                    assert_equals(&unique_count, &offset_key);
                    assert_equals(&offset_hash.size(), &(count + 1));
                    unique_count += 1;
                } else {
                    Assert::assert_false(strings.insert(str.clone()));
                    Assert::assert_true((-key) - 1 < count);
                    assert_equals(&str, &get_string(hash, pool, (-key) - 1));
                    assert_equals(&count, &hash.size());
                    let offset_key = offset_hash.add_by_pool_offset(hash.byte_start((-key) - 1));
                    Assert::assert_true((-offset_key) - 1 < count);
                    assert_equals(&str, &get_string(hash, pool, (-offset_key) - 1));
                    assert_equals(&count, &hash.size());
                }
            }

            assert_all_in(&strings, hash, pool)?;
            for string in &strings {
                r#ref.copy_chars(string);
                let key = hash.add(pool, r#ref.get().slice())?;
                let bytes_ref = get(&offset_hash, pool, (-key) - 1);
                assert_equals(&r#ref.get(), &bytes_ref);
            }

            hash.clear();
            assert_equals(&0, &hash.size());
            offset_hash.clear();
            assert_equals(&0, &offset_hash.size());
            hash.reinit(); // init for the next round
            offset_hash.reinit();
        }
        Ok(())
    }
}

/// `assertAllIn(strings, hash)`.
fn assert_all_in(
    strings: &HashSet<String>,
    hash: &mut BytesRefHash,
    pool: &mut ByteBlockPool,
) -> Result<()> {
    let mut r#ref = BytesRefBuilder::new();
    let count = hash.size();
    for string in strings {
        r#ref.copy_chars(string);
        let key = hash.add(pool, r#ref.get().slice())?; // add again to check duplicates
        assert_equals(string, &get_string(hash, pool, (-key) - 1));
        assert_equals(&count, &hash.size());
        Assert::assert_true_msg(
            &format!("key: {key} count: {count} string: {string}"),
            key < count,
        );
    }
    Ok(())
}
