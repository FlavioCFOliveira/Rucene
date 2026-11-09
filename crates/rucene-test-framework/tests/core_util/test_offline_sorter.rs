//! Port of `lucene/core/src/test/org/apache/lucene/util/TestOfflineSorter.java`.
//!
//! Methods not declared (each carries a `PENDING` note at its place below):
//!
//! * `testIntermediateMerges`, `testSmallRandom`, `testLargerRandom`
//!   (`@Nightly`) and `testFixedLengthHeap` (`@Nightly`) build the sorter with
//!   `OfflineSorter(dir, prefix, comparator, ramBufferSize, maxTempfiles,
//!   valueLength, ExecutorService exec, int maxPartitionsInRAM)`; Rucene's
//!   `OfflineSorter` has no executor and no `maxPartitionsInRAM` (it always
//!   sorts on the calling thread, one partition in RAM).
//! * `testBitFlippedOnInput2` and `testBitFlippedOnPartition2` override
//!   `CorruptingIndexOutput.corruptFile()`, which Rucene's
//!   `CorruptingIndexOutput` (a struct) cannot have overridden.
//! * `testOverNexting` overrides `OfflineSorter.getReader(ChecksumIndexInput,
//!   String)`, which Rucene's `OfflineSorter` (a struct) cannot have
//!   overridden.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::codecs::codec_util;
use rucene::error::Result;
use rucene::store::{
    BufferedChecksumIndexInput, ByteBuffersDirectory, Directory, IOContext, IndexInput,
    IndexOutput, Lock, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use rucene::util::offline_sorter::{
    BufferSize, ByteSequencesWriter, OfflineSorter, OfflineSorterComparator, SortInfo,
    MAX_TEMPFILES,
};
use rucene_test_framework::store::corrupting_index_output::CorruptingIndexOutput;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestOfflineSorter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestOfflineSorter",
        vec![
            test("testEmpty", test_empty),
            test("testSingleLine", test_single_line),
            test("testRamBuffer", test_ram_buffer),
            test("testThreadSafety", test_thread_safety),
            test("testBitFlippedOnInput1", test_bit_flipped_on_input1),
            test("testBitFlippedOnPartition1", test_bit_flipped_on_partition1),
            test(
                "testFixedLengthLiesLiesLies",
                test_fixed_length_lies_lies_lies,
            ),
            test("testInvalidFixedLength", test_invalid_fixed_length),
        ],
    )
}

// `setUp()`: `tempDir = createTempDir("mergesort")`; `tearDown()`:
// `IOUtils.rm(tempDir)`. The field is never read by a test; the temp dir
// rule deletes what `createTempDir` creates.

fn test_empty() -> Result<()> {
    let _temp_dir = LuceneTestCase::create_temp_dir_prefix("mergesort");
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    check_sort(&d, &mut OfflineSorter::new(Arc::clone(&d), "foo")?, vec![])?;
    drop(d);
    dir.close()
}

fn test_single_line() -> Result<()> {
    let _temp_dir = LuceneTestCase::create_temp_dir_prefix("mergesort");
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    check_sort(
        &d,
        &mut OfflineSorter::new(Arc::clone(&d), "foo")?,
        vec![b"Single line only.".to_vec()],
    )?;
    drop(d);
    dir.close()
}

// PENDING(member): `randomExecutorServiceOrNull()`, `testIntermediateMerges`,
// `testSmallRandom`, `testLargerRandom` (`@Nightly`) and
// `testFixedLengthHeap` (`@Nightly`) need the `OfflineSorter` constructor
// taking an `ExecutorService` and `maxPartitionsInRAM`
// (`OfflineSorter.java:225-262`), which Rucene's `OfflineSorter` lacks.

fn generate_random(how_much_data_in_bytes: i32) -> Vec<Vec<u8>> {
    let mut how_much = how_much_data_in_bytes;
    let mut data: Vec<Vec<u8>> = Vec::new();
    while how_much > 0 {
        let mut current = vec![0u8; random().next_int_bound(256) as usize];
        random().next_bytes(&mut current);
        how_much -= current.len() as i32;
        data.push(current);
    }
    data
}

// Generates same data every time:
fn generate_fixed(how_much_data_in_bytes: i32) -> Vec<Vec<u8>> {
    let mut how_much = how_much_data_in_bytes;
    let mut data: Vec<Vec<u8>> = Vec::new();
    let mut length = 256usize;
    let mut counter: u8 = 0;
    while how_much > 0 {
        let mut current = vec![0u8; length];
        for b in current.iter_mut() {
            *b = counter;
            counter = counter.wrapping_add(1);
        }
        how_much -= current.len() as i32;
        data.push(current);
        length -= 1;
        if length <= 128 {
            length = 256;
        }
    }
    data
}

/// `unsignedByteOrderComparator`.
fn unsigned_byte_order_comparator(left: &[u8], right: &[u8]) -> std::cmp::Ordering {
    let max = left.len().min(right.len());
    for i in 0..max {
        let diff = left[i] as i32 - right[i] as i32;
        if diff != 0 {
            return diff.cmp(&0);
        }
    }
    (left.len() as i32 - right.len() as i32).cmp(&0)
}

/// Check sorting data on an instance of `OfflineSorter`.
fn check_sort(
    dir: &Arc<dyn Directory>,
    sorter: &mut OfflineSorter,
    mut data: Vec<Vec<u8>>,
) -> Result<SortInfo> {
    let unsorted = dir.create_temp_output("unsorted", "tmp", &*DEFAULT_IO_CONTEXT)?;
    let unsorted_name = unsorted.name().to_string();
    write_all(unsorted, &data)?;

    let golden = dir.create_temp_output("golden", "tmp", &*DEFAULT_IO_CONTEXT)?;
    let golden_name = golden.name().to_string();
    // `Arrays.sort(Object[], Comparator)`: a stable sort.
    data.sort_by(|a, b| unsigned_byte_order_comparator(a, b));
    write_all(golden, &data)?;

    let sorted = sorter.sort(&unsorted_name)?;
    assert_files_identical(dir, &golden_name, &sorted)?;

    Ok(sorter.sort_info().clone())
}

/// Make sure two files are byte-byte identical.
fn assert_files_identical(dir: &Arc<dyn Directory>, golden: &str, sorted: &str) -> Result<()> {
    let num_bytes = dir.file_length(golden)?;
    assert_equals(&num_bytes, &dir.file_length(sorted)?);

    let mut buf1 = vec![0u8; 64 * 1024];
    let mut buf2 = vec![0u8; 64 * 1024];
    let mut in1 = dir.open_input(golden, &*READONCE_IO_CONTEXT)?;
    let mut in2 = dir.open_input(sorted, &*READONCE_IO_CONTEXT)?;
    let mut left = num_bytes;
    let body = (|| -> Result<()> {
        while left > 0 {
            let chunk = (buf1.len() as i64).min(left) as usize;
            left -= chunk as i64;
            in1.read_bytes(&mut buf1, 0, chunk)?;
            in2.read_bytes(&mut buf2, 0, chunk)?;
            for i in 0..chunk {
                assert_equals(&buf1[i], &buf2[i]);
            }
        }
        Ok(())
    })();
    let c1 = in1.close();
    let c2 = in2.close();
    body.and(c1).and(c2)
}

/// NOTE: closes the provided `IndexOutput`
fn write_all(out: Box<dyn IndexOutput>, data: &[Vec<u8>]) -> Result<()> {
    let mut w = ByteSequencesWriter::new(out);
    for datum in data {
        w.write(datum)?;
    }
    codec_util::write_footer(w.out())?;
    w.close()
}

fn test_ram_buffer() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(10000);
    for _ in 0..num_iters {
        BufferSize::megabytes(1 + random().next_int_bound(2047) as i64)?;
    }
    BufferSize::megabytes(2047)?;
    BufferSize::megabytes(1)?;

    expect_throws("java.lang.IllegalArgumentException", || {
        BufferSize::megabytes(2048)
    });
    expect_throws("java.lang.IllegalArgumentException", || {
        BufferSize::megabytes(0)
    });
    expect_throws("java.lang.IllegalArgumentException", || {
        BufferSize::megabytes(-1)
    });
    Ok(())
}

fn test_thread_safety() -> Result<()> {
    let _temp_dir = LuceneTestCase::create_temp_dir_prefix("mergesort");
    let num_threads = TestUtil::next_int(&*random(), 4, 10);
    let failed = Arc::new(AtomicBool::new(false));
    let iters = LuceneTestCase::at_least(200);
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let mut threads = Vec::with_capacity(num_threads as usize);
    for i in 0..num_threads {
        let thread_id = i;
        let failed = Arc::clone(&failed);
        let d = Arc::clone(&d);
        let t = Thread::unnamed(move || {
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                let mut iter = 0;
                while iter < iters && !failed.load(Ordering::SeqCst) {
                    check_sort(
                        &d,
                        &mut OfflineSorter::new(
                            Arc::clone(&d),
                            &format!("foo_{thread_id}_{iter}"),
                        )?,
                        generate_random(1024),
                    )?;
                    iter += 1;
                }
                Ok(())
            }));
            match run {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    failed.store(true, Ordering::SeqCst);
                    rethrow_java(e)
                }
                Err(panic) => {
                    failed.store(true, Ordering::SeqCst);
                    std::panic::resume_unwind(panic)
                }
            }
        });
        t.start();
        threads.push(t);
    }
    for thread in &threads {
        let _ = thread.join();
    }
    drop(d);
    dir.close()?;
    Assert::assert_false(failed.load(Ordering::SeqCst));
    Ok(())
}

/// The anonymous `FilterDirectory` subclasses of the `testBitFlipped*` tests:
/// every method delegates to `in`, except `createTempOutput`, which wraps the
/// output in a `CorruptingIndexOutput` when `corrupt` says so.
struct CorruptingFilterDirectory {
    r#in: Arc<dyn Directory>,
    dir0: Arc<dyn Directory>,
    byte_to_corrupt: i64,
    /// `true` when `createTempOutput(prefix, suffix, …)` must corrupt.
    corrupt: fn(&CorruptingFilterDirectory, &str, &str) -> bool,
    /// The `boolean corrupted` field of the partition variants.
    corrupted: Mutex<bool>,
}

impl Directory for CorruptingFilterDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        let out = self.r#in.create_temp_output(prefix, suffix, context)?;
        if (self.corrupt)(self, prefix, suffix) {
            Ok(Box::new(CorruptingIndexOutput::new(
                Arc::clone(&self.dir0),
                self.byte_to_corrupt,
                out,
            )))
        } else {
            Ok(out)
        }
    }
    fn sync(&self, names: &[String]) -> Result<()> {
        self.r#in.sync(names)
    }
    fn sync_metadata(&self) -> Result<()> {
        self.r#in.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.r#in.rename(source, dest)
    }
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.r#in.open_input(name, context)
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.r#in.open_checksum_input(name)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }
    fn close(&mut self) -> Result<()> {
        // `FilterDirectory.close()` closes `in`; the test closes `dir0` itself
        // (`try (Directory dir0 = newMockDirectory())`) and never closes `dir`.
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.r#in.get_pending_deletions()
    }
}

/// `prefix.equals("unsorted")`.
fn corrupt_unsorted(_d: &CorruptingFilterDirectory, prefix: &str, _suffix: &str) -> bool {
    prefix == "unsorted"
}

/// `corrupted == false && suffix.equals("sort")`, setting `corrupted`.
fn corrupt_first_sort(d: &CorruptingFilterDirectory, _prefix: &str, suffix: &str) -> bool {
    let mut corrupted = d.corrupted.lock().unwrap_or_else(|e| e.into_inner());
    if !*corrupted && suffix == "sort" {
        *corrupted = true;
        true
    } else {
        false
    }
}

/// Make sure corruption on the incoming (unsorted) file is caught, even if the
/// corruption didn't confuse OfflineSorter!
fn test_bit_flipped_on_input1() -> Result<()> {
    let _temp_dir = LuceneTestCase::create_temp_dir_prefix("mergesort");
    let mut dir0_owner = LuceneTestCase::new_mock_directory();
    let dir0 = dir0_owner.as_directory();
    let dir: Arc<dyn Directory> = Arc::new(CorruptingFilterDirectory {
        r#in: Arc::clone(&dir0),
        dir0: Arc::clone(&dir0),
        byte_to_corrupt: 22,
        corrupt: corrupt_unsorted,
        corrupted: Mutex::new(false),
    });

    let unsorted = dir.create_temp_output("unsorted", "tmp", &*DEFAULT_IO_CONTEXT)?;
    let unsorted_name = unsorted.name().to_string();
    write_all(unsorted, &generate_fixed(10 * 1024))?;

    let e = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        OfflineSorter::new(Arc::clone(&dir), "foo")?.sort(&unsorted_name)
    });
    Assert::assert_true(
        e.message()
            .unwrap_or_default()
            .contains("checksum failed (hardware problem?)"),
    );
    drop(dir);
    drop(dir0);
    dir0_owner.close()
}

// PENDING(member): `testBitFlippedOnInput2` overrides
// `CorruptingIndexOutput.corruptFile()` in an anonymous subclass; Rucene's
// `CorruptingIndexOutput` is a struct whose `corrupt_file` cannot be
// overridden.

/// Make sure corruption on a temp file (partition) is caught, even if the
/// corruption didn't confuse OfflineSorter!
fn test_bit_flipped_on_partition1() -> Result<()> {
    let _temp_dir = LuceneTestCase::create_temp_dir_prefix("mergesort");
    let mut dir0_owner = LuceneTestCase::new_mock_directory();
    let dir0 = dir0_owner.as_directory();
    let dir: Arc<dyn Directory> = Arc::new(CorruptingFilterDirectory {
        r#in: Arc::clone(&dir0),
        dir0: Arc::clone(&dir0),
        byte_to_corrupt: 544677,
        corrupt: corrupt_first_sort,
        corrupted: Mutex::new(false),
    });

    let unsorted = dir.create_temp_output("unsorted", "tmp", &*DEFAULT_IO_CONTEXT)?;
    let unsorted_name = unsorted.name().to_string();
    write_all(
        unsorted,
        &generate_fixed((rucene::util::offline_sorter::MB * 3) as i32),
    )?;

    // `new OfflineSorter(dir, "foo", DEFAULT_COMPARATOR, BufferSize.megabytes(1),
    // 10, -1, null, 0)`: no executor, which is the only form Rucene has.
    let e = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        OfflineSorter::with_options(
            Arc::clone(&dir),
            "foo",
            OfflineSorterComparator::Natural,
            BufferSize::megabytes(1)?,
            10,
            -1,
        )?
        .sort(&unsorted_name)
    });
    Assert::assert_true(
        e.message()
            .unwrap_or_default()
            .contains("checksum failed (hardware problem?)"),
    );
    drop(dir);
    drop(dir0);
    dir0_owner.close()
}

// PENDING(member): `testBitFlippedOnPartition2` overrides
// `CorruptingIndexOutput.corruptFile()` (see `testBitFlippedOnInput2`).

fn test_fixed_length_lies_lies_lies() -> Result<()> {
    let _temp_dir = LuceneTestCase::create_temp_dir_prefix("mergesort");
    // Make sure OfflineSorter catches me if I lie about the fixed value length:
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let out = d.create_temp_output("unsorted", "tmp", &*DEFAULT_IO_CONTEXT)?;
    let out_name = out.name().to_string();
    {
        let mut w = ByteSequencesWriter::new(out);
        let mut bytes = [0u8; 4];
        random().next_bytes(&mut bytes);
        w.write(&bytes)?;
        codec_util::write_footer(w.out())?;
        w.close()?;
    }

    // `null` executor, `0` partitions: the form Rucene's `with_options` is.
    let mut sorter = OfflineSorter::with_options(
        Arc::clone(&d),
        "foo",
        OfflineSorterComparator::Natural,
        BufferSize::megabytes(4)?,
        MAX_TEMPFILES,
        8,
    )?;
    let e = expect_throws("java.lang.IllegalArgumentException", || {
        sorter.sort(&out_name)
    });
    assert_equals(
        &Some("value length is 4 but is supposed to always be 8"),
        &e.message(),
    );
    drop(d);
    dir.close()
}

// PENDING(member): `testOverNexting` overrides
// `OfflineSorter.getReader(ChecksumIndexInput, String)` in an anonymous
// subclass; Rucene's `OfflineSorter` is a struct whose `get_reader` cannot be
// overridden.

fn test_invalid_fixed_length() -> Result<()> {
    let _temp_dir = LuceneTestCase::create_temp_dir_prefix("mergesort");
    // Java passes a `null` directory, which the constructor never reads before
    // the `valueLength` check; Rucene's constructor takes a directory, so an
    // unused empty one stands in for it.
    let unused: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let e = expect_throws("java.lang.IllegalArgumentException", || {
        OfflineSorter::with_options(
            Arc::clone(&unused),
            "foo",
            OfflineSorterComparator::Natural,
            BufferSize::megabytes(1)?,
            MAX_TEMPFILES,
            0,
        )
    });
    assert_equals(
        &Some("valueLength must be 1 .. 32767; got: 0"),
        &e.message(),
    );
    let e = expect_throws("java.lang.IllegalArgumentException", || {
        OfflineSorter::with_options(
            Arc::clone(&unused),
            "foo",
            OfflineSorterComparator::Natural,
            BufferSize::megabytes(1)?,
            MAX_TEMPFILES,
            i32::MAX,
        )
    });
    assert_equals(
        &Some("valueLength must be 1 .. 32767; got: 2147483647"),
        &e.message(),
    );
    Ok(())
}
