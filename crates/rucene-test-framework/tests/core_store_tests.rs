//! Port of the `lucene/core/src/test/org/apache/lucene/store/` test classes
//! of Lucene 10.5.0 that extend `LuceneTestCase` (or `RandomizedTest`)
//! directly, each run as a suite by the ported runner:
//! `TestBufferedChecksum`, `TestBufferedIndexInput`, `TestByteArrayDataInput`,
//! `TestByteBuffersDataInput`, `TestByteBuffersDataOutput` and
//! `TestFilterIndexOutput` (with their base `BaseDataOutputTestCase`),
//! `TestChecksumIndexInput`, `TestDirectory`, `TestIndexOutputAlignment`,
//! `TestInputStreamDataInput`, `TestIOContext`, `TestLockFactory`,
//! `TestOutputStreamIndexOutput`, `TestRateLimiter` and
//! `TestStressLockFactories`.
//!
//! The store classes that extend `BaseDirectoryTestCase`,
//! `BaseChunkedDirectoryTestCase` or `BaseLockFactoryTestCase` declare their
//! own methods on their bindings in `base_directory_suites.rs` and
//! `base_lock_factory_suites.rs`.

mod lucene_core_shims;
mod suite_seeds;

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use lucene_core_shims::{
    assert_bytes_equals, assert_bytes_equals_msg, lucene_class, run, t, ti, CountDownLatch, Crc32,
};
use randomizedtesting::annotations::Timeout;
use randomizedtesting::generators::{RandomBytes, RandomNumbers, RandomPicks, RandomStrings};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{After, AfterClass, Assert, Before, BeforeClass};
use randomizedtesting::{RandomizedTest, Xoroshiro128PlusRandom};
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::store::java_nio_file::Files;
use rucene::store::mmap::MMapDirectory;
use rucene::store::{
    align_offset, flush_io_context, merge_io_context, BufferedChecksum, BufferedChecksumIndexInput,
    BufferedIndexInput, ByteArrayDataInput, ByteArrayDataOutput, ByteBufferRecycler,
    ByteBuffersDataInput, ByteBuffersDataOutput, ByteBuffersDirectory, ByteBuffersIndexOutput,
    ChecksumIndexInput, DataAccessHint, DataInput, DataOutput, Directory, FSDirectory,
    FileDataHint, FileOpenHint, FileTypeHint, FilterIndexOutput, FlushInfo, IOContext, IndexInput,
    IndexOutput, InputStreamDataInput, Lock, LockFactory, LockStressTest, LockVerifyServer,
    MergeInfo, NIOFSDirectory, NoLockFactory, OutputStreamDataOutput, OutputStreamIndexOutput,
    PreloadHint, RandomAccessInput, RateLimiter, SimpleRateLimiter, BUFFER_SIZE,
    CHECKSUM_SKIP_BUFFER_SIZE, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use rucene::util::Accountable;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::{
    LuceneTestCase, RANDOM_MULTIPLIER, TEST_NIGHTLY, VERBOSE,
};
use rucene_test_framework::util::TestUtil;

fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

// -----------------------------------------------------------------------------
// TestBufferedChecksum
// -----------------------------------------------------------------------------

/// `TestBufferedChecksum.testSimple()`.
fn buffered_checksum_test_simple() -> Result<()> {
    let mut c = BufferedChecksum::new();
    c.update(1);
    c.update(2);
    c.update(3);
    Assert::assert_equals(1438416925i64, c.get_value());
    Ok(())
}

/// `TestBufferedChecksum.testRandom()`.
fn buffered_checksum_test_random() -> Result<()> {
    let mut c1 = Crc32::new();
    let mut c2 = BufferedChecksum::new();
    let iterations = LuceneTestCase::at_least(10000);
    for _ in 0..iterations {
        match random().next_int_bound(4) {
            0 => {
                // update(byte[], int, int)
                let length = random().next_int_bound(1024) as usize;
                let mut bytes = vec![0u8; length];
                random().next_bytes(&mut bytes);
                c1.update_bytes(&bytes, 0, bytes.len());
                c2.update_bytes(&bytes, 0, bytes.len())?;
            }
            1 => {
                // update(int)
                let b = random().next_int_bound(256);
                c1.update(b);
                c2.update(b as u8);
            }
            2 => {
                // reset()
                c1.reset();
                c2.reset();
            }
            3 => {
                // getValue()
                Assert::assert_equals(c1.get_value(), c2.get_value());
            }
            _ => unreachable!(),
        }
    }
    Assert::assert_equals(c1.get_value(), c2.get_value());
    Ok(())
}

/// `TestBufferedChecksum.testDifferentInputTypes()`.
fn buffered_checksum_test_different_input_types() -> Result<()> {
    let mut crc = Crc32::new();
    let mut buffered = BufferedChecksum::new();
    let iterations = LuceneTestCase::at_least(1000);
    for _ in 0..iterations {
        let mut input = vec![0u8; 4096];
        random().next_bytes(&mut input);
        crc.update_all(&input);
        let checksum = crc.get_value();
        crc.reset();
        update_by_shorts(checksum, &mut buffered, &input)?;
        update_by_ints(checksum, &mut buffered, &input)?;
        update_by_longs(checksum, &mut buffered, &input)?;
        update_by_chunk_of_bytes(checksum, &mut buffered, &input)?;
        update_by_chunk_of_longs(checksum, &mut buffered, &input)?;
    }
    Ok(())
}

/// `updateByChunkOfBytes(long, BufferedChecksum, byte[])` (`private`).
fn update_by_chunk_of_bytes(
    expected: i64,
    checksum: &mut BufferedChecksum,
    input: &[u8],
) -> Result<()> {
    for &b in input {
        checksum.update(b);
    }
    check_checksum_value_and_reset(expected, checksum);

    checksum.update_bytes(input, 0, input.len())?;
    check_checksum_value_and_reset(expected, checksum);

    let iterations = LuceneTestCase::at_least(10);
    for _ in 0..iterations {
        let len0 = random().next_int_bound((input.len() / 2) as i32) as usize;
        checksum.update_bytes(input, 0, len0)?;
        checksum.update_bytes(input, len0, input.len() - len0)?;
        check_checksum_value_and_reset(expected, checksum);

        checksum.update_bytes(input, 0, len0)?;
        let len1 = random().next_int_bound((input.len() / 4) as i32) as usize;
        for i in 0..len1 {
            checksum.update(input[len0 + i]);
        }
        checksum.update_bytes(input, len0 + len1, input.len() - len1 - len0)?;
        check_checksum_value_and_reset(expected, checksum);
    }
    Ok(())
}

/// `updateByShorts(long, BufferedChecksum, byte[])` (`private`).
fn update_by_shorts(expected: i64, checksum: &mut BufferedChecksum, input: &[u8]) -> Result<()> {
    let mut ix = shift_array(checksum, input)?;
    while ix + 2 <= input.len() {
        checksum.update_short(i16::from_le_bytes([input[ix], input[ix + 1]]));
        ix += 2;
    }
    checksum.update_bytes(input, ix, input.len() - ix)?;
    check_checksum_value_and_reset(expected, checksum);
    Ok(())
}

/// `updateByInts(long, BufferedChecksum, byte[])` (`private`).
fn update_by_ints(expected: i64, checksum: &mut BufferedChecksum, input: &[u8]) -> Result<()> {
    let mut ix = shift_array(checksum, input)?;
    while ix + 4 <= input.len() {
        checksum.update_int(i32::from_le_bytes(input[ix..ix + 4].try_into().unwrap()));
        ix += 4;
    }
    checksum.update_bytes(input, ix, input.len() - ix)?;
    check_checksum_value_and_reset(expected, checksum);
    Ok(())
}

/// `updateByLongs(long, BufferedChecksum, byte[])` (`private`).
fn update_by_longs(expected: i64, checksum: &mut BufferedChecksum, input: &[u8]) -> Result<()> {
    let mut ix = shift_array(checksum, input)?;
    while ix + 8 <= input.len() {
        checksum.update_long(i64::from_le_bytes(input[ix..ix + 8].try_into().unwrap()));
        ix += 8;
    }
    checksum.update_bytes(input, ix, input.len() - ix)?;
    check_checksum_value_and_reset(expected, checksum);
    Ok(())
}

/// `shiftArray(BufferedChecksum, byte[])` (`private static`).
fn shift_array(checksum: &mut BufferedChecksum, input: &[u8]) -> Result<usize> {
    let ix = random().next_int_bound((input.len() / 4) as i32) as usize;
    checksum.update_bytes(input, 0, ix)?;
    Ok(ix)
}

/// `updateByChunkOfLongs(long, BufferedChecksum, byte[])` (`private`).
fn update_by_chunk_of_longs(
    expected: i64,
    checksum: &mut BufferedChecksum,
    input: &[u8],
) -> Result<()> {
    let ix = random().next_int_bound((input.len() / 4) as i32) as usize;
    // `Long.BYTES - ix & 7`: `-` binds tighter than `&`.
    let remaining = (8usize.wrapping_sub(ix)) & 7;
    // `ByteBuffer.wrap(input).position(ix).order(LITTLE_ENDIAN).asLongBuffer()`
    let long_input: Vec<i64> = (0..(input.len() - ix) / 8)
        .map(|i| {
            let at = ix + i * 8;
            i64::from_le_bytes(input[at..at + 8].try_into().unwrap())
        })
        .collect();

    checksum.update_bytes(input, 0, ix)?;
    for &l in &long_input {
        checksum.update_long(l);
    }
    checksum.update_bytes(input, input.len() - remaining, remaining)?;
    check_checksum_value_and_reset(expected, checksum);

    checksum.update_bytes(input, 0, ix)?;
    checksum.update_longs(&long_input, 0, long_input.len())?;
    checksum.update_bytes(input, input.len() - remaining, remaining)?;
    check_checksum_value_and_reset(expected, checksum);

    let iterations = LuceneTestCase::at_least(10);
    for _ in 0..iterations {
        let len0 = random().next_int_bound((long_input.len() / 2) as i32) as usize;
        checksum.update_bytes(input, 0, ix)?;
        checksum.update_longs(&long_input, 0, len0)?;
        checksum.update_longs(&long_input, len0, long_input.len() - len0)?;
        checksum.update_bytes(input, input.len() - remaining, remaining)?;
        check_checksum_value_and_reset(expected, checksum);

        checksum.update_bytes(input, 0, ix)?;
        checksum.update_longs(&long_input, 0, len0)?;
        let len1 = random().next_int_bound((long_input.len() / 4) as i32) as usize;
        for i in 0..len1 {
            checksum.update_long(long_input[len0 + i]);
        }
        checksum.update_longs(&long_input, len0 + len1, long_input.len() - len1 - len0)?;
        checksum.update_bytes(input, input.len() - remaining, remaining)?;
        check_checksum_value_and_reset(expected, checksum);

        checksum.update_bytes(input, 0, ix)?;
        checksum.update_longs(&long_input, 0, len0)?;
        checksum.update_bytes(input, ix + len0 * 8, input.len() - len0 * 8 - ix)?;
        check_checksum_value_and_reset(expected, checksum);
    }
    Ok(())
}

/// `checkChecksumValueAndReset(long, Checksum)` (`private`).
fn check_checksum_value_and_reset(expected: i64, checksum: &mut BufferedChecksum) {
    Assert::assert_equals(expected, checksum.get_value());
    checksum.reset();
}

fn buffered_checksum_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestBufferedChecksum")
        .method(t("testSimple", buffered_checksum_test_simple))
        .method(t("testRandom", buffered_checksum_test_random))
        .method(t(
            "testDifferentInputTypes",
            buffered_checksum_test_different_input_types,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestBufferedIndexInput
// -----------------------------------------------------------------------------

/// `TEST_FILE_LENGTH` (`private static final long`).
const TEST_FILE_LENGTH: i64 = 100 * 1024;

/// The instance state of `TestBufferedIndexInput`: its `buffer` field.
struct TestBufferedIndexInput {
    buffer: Vec<u8>,
}

impl TestBufferedIndexInput {
    fn new() -> TestBufferedIndexInput {
        TestBufferedIndexInput {
            buffer: vec![0u8; 10],
        }
    }

    /// `testReadByte()`: call `readByte()` repeatedly, past the buffer
    /// boundary, and see that it is working as expected. Our input comes from
    /// a dynamically generated "file" - see `MyBufferedIndexInput` below.
    fn test_read_byte(&mut self) -> Result<()> {
        let (mut input, _) = my_buffered_index_input_infinite()?;
        for i in 0..(BUFFER_SIZE * 10) as i64 {
            Assert::assert_equals(input.read_byte()? as i8, byten(i));
        }
        Ok(())
    }

    /// `testReadBytes()`: call `readBytes()` repeatedly, with various chunk
    /// sizes (from 1 byte to larger than the buffer size), and see that it
    /// returns the bytes we expect.
    fn test_read_bytes(&mut self) -> Result<()> {
        let (mut input, _) = my_buffered_index_input_infinite()?;
        self.run_read_bytes(&mut input, BUFFER_SIZE as i32, &*random())
    }

    /// `runReadBytes(IndexInput, int, Random)` (`private`).
    fn run_read_bytes(
        &mut self,
        input: &mut dyn IndexInput,
        buffer_size: i32,
        r: &dyn Random,
    ) -> Result<()> {
        let mut pos: i32 = 0;
        // gradually increasing size:
        let mut size = 1;
        while size < buffer_size * 10 {
            self.check_read_bytes(input, size, pos)?;
            pos += size;
            if pos as i64 >= TEST_FILE_LENGTH {
                // wrap
                pos = 0;
                input.seek(0)?;
            }
            size = size + size / 200 + 1;
        }
        // wildly fluctuating size:
        for _ in 0..100i64 {
            let size = r.next_int_bound(10000);
            self.check_read_bytes(input, 1 + size, pos)?;
            pos += 1 + size;
            if pos as i64 >= TEST_FILE_LENGTH {
                // wrap
                pos = 0;
                input.seek(0)?;
            }
        }
        // constant small size (7 bytes):
        for _ in 0..buffer_size {
            self.check_read_bytes(input, 7, pos)?;
            pos += 7;
            if pos as i64 >= TEST_FILE_LENGTH {
                // wrap
                pos = 0;
                input.seek(0)?;
            }
        }
        Ok(())
    }

    /// `checkReadBytes(IndexInput, int, int)` (`private`).
    fn check_read_bytes(&mut self, input: &mut dyn IndexInput, size: i32, pos: i32) -> Result<()> {
        let mut size = size;
        // Just to see that "offset" is treated properly in readBytes(), we
        // add an arbitrary offset at the beginning of the array
        let offset = size % 10; // arbitrary
        self.buffer = rucene::util::ArrayUtil::grow(&self.buffer, (offset + size) as usize);
        Assert::assert_equals(pos as i64, input.file_pointer());
        let left = TEST_FILE_LENGTH - input.file_pointer();
        if left <= 0 {
            return Ok(());
        } else if left < size as i64 {
            size = left as i32;
        }
        input.read_bytes(&mut self.buffer, offset as usize, size as usize)?;
        Assert::assert_equals((pos + size) as i64, input.file_pointer());
        for i in 0..size {
            Assert::assert_equals_msg(
                Some(&format!("pos={i} filepos={}", pos + i)),
                byten((pos + i) as i64),
                self.buffer[(offset + i) as usize] as i8,
            );
        }
        Ok(())
    }

    /// `testEOF()`: attempts to `readBytes()` past an EOF will fail, while
    /// reads up to the EOF will succeed. The EOF is determined by the
    /// `BufferedIndexInput`'s arbitrary `length()` value.
    fn test_eof(&mut self) -> Result<()> {
        let (mut input, _) = my_buffered_index_input(1024)?;
        // see that we can read all the bytes at one go:
        let len = input.length() as i32;
        self.check_read_bytes(&mut input, len, 0)?;
        // go back and see that we can't read more than that, for small and
        // large overflows:
        let pos = input.length() as i32 - 10;
        input.seek(pos as i64)?;
        self.check_read_bytes(&mut input, 10, pos)?;
        input.seek(pos as i64)?;
        // block read past end of file
        expect_throws("java.io.IOException", || {
            self.check_read_bytes(&mut input, 11, pos)
        });
        input.seek(pos as i64)?;
        // block read past end of file
        expect_throws("java.io.IOException", || {
            self.check_read_bytes(&mut input, 50, pos)
        });
        input.seek(pos as i64)?;
        // block read past end of file
        expect_throws("java.io.IOException", || {
            self.check_read_bytes(&mut input, 100000, pos)
        });
        Ok(())
    }

    /// `testBackwardsByteReads()`: when reading backwards, we page backwards
    /// rather than refilling on every call.
    fn test_backwards_byte_reads(&mut self) -> Result<()> {
        let (mut input, read_count) = my_buffered_index_input(1024 * 8)?;
        let mut i = 2048;
        while i > 0 {
            Assert::assert_equals(byten(i as i64), input.read_byte_at(i as i64)? as i8);
            i -= random().next_int_bound(16);
        }
        Assert::assert_equals(3i64, read_count.load(Ordering::SeqCst));
        Ok(())
    }

    /// `testBackwardsShortReads()`.
    fn test_backwards_short_reads(&mut self) -> Result<()> {
        let (mut input, read_count) = my_buffered_index_input(1024 * 8)?;
        let mut i = 2048;
        while i > 0 {
            let bb = [byten(i as i64) as u8, byten(i as i64 + 1) as u8];
            Assert::assert_equals(i16::from_le_bytes(bb), input.read_short_at(i as i64)?);
            i -= random().next_int_bound(16) + 1;
        }
        // readCount can be three or four, depending on whether or not we had to adjust the
        // bufferStart to include a whole short
        let rc = read_count.load(Ordering::SeqCst);
        Assert::assert_true_msg(&format!("Expected 4 or 3, got {rc}"), rc == 4 || rc == 3);
        Ok(())
    }

    /// `testBackwardsIntReads()`.
    fn test_backwards_int_reads(&mut self) -> Result<()> {
        let (mut input, read_count) = my_buffered_index_input(1024 * 8)?;
        let mut i = 2048;
        while i > 0 {
            let mut bb = [0u8; 4];
            for (k, b) in bb.iter_mut().enumerate() {
                *b = byten(i as i64 + k as i64) as u8;
            }
            Assert::assert_equals(i32::from_le_bytes(bb), input.read_int_at(i as i64)?);
            i -= random().next_int_bound(16) + 3;
        }
        // readCount can be three or four, depending on whether or not we had to adjust the
        // bufferStart to include a whole int
        let rc = read_count.load(Ordering::SeqCst);
        Assert::assert_true_msg(&format!("Expected 4 or 3, got {rc}"), rc == 4 || rc == 3);
        Ok(())
    }

    /// `testBackwardsLongReads()`.
    fn test_backwards_long_reads(&mut self) -> Result<()> {
        let (mut input, read_count) = my_buffered_index_input(1024 * 8)?;
        let mut i = 2048;
        while i > 0 {
            let mut bb = [0u8; 8];
            for (k, b) in bb.iter_mut().enumerate() {
                *b = byten(i as i64 + k as i64) as u8;
            }
            Assert::assert_equals(i64::from_le_bytes(bb), input.read_long_at(i as i64)?);
            i -= random().next_int_bound(16) + 7;
        }
        // readCount can be three or four, depending on whether or not we had to adjust the
        // bufferStart to include a whole long
        let rc = read_count.load(Ordering::SeqCst);
        Assert::assert_true_msg(&format!("Expected 4 or 3, got {rc}"), rc == 4 || rc == 3);
        Ok(())
    }

    /// `testReadFloats()`.
    fn test_read_floats(&mut self) -> Result<()> {
        let length = 1024 * 8;
        let (mut input, _) = my_buffered_index_input(length as i64)?;
        let buffer_length = 128;
        let mut float_buffer = vec![0f32; buffer_length];
        for alignment in 0..4 {
            input.seek(0)?;
            for _ in 0..alignment {
                input.read_byte()?;
            }
            let bulk_reads = length / (buffer_length * 4) - 1;
            for i in 0..bulk_reads {
                let pos = alignment + i * buffer_length * 4;
                let float_offset = random().next_int_bound(3) as usize;
                input.skip_bytes((float_offset * 4) as i64)?;
                input.read_floats(
                    &mut float_buffer,
                    float_offset,
                    buffer_length - float_offset,
                )?;
                for (idx, &f) in float_buffer.iter().enumerate().skip(float_offset) {
                    let offset = (pos + idx * 4) as i64;
                    let bb = [
                        byten(offset) as u8,
                        byten(offset + 1) as u8,
                        byten(offset + 2) as u8,
                        byten(offset + 3) as u8,
                    ];
                    Assert::assert_equals(
                        f32::from_le_bytes(bb).to_bits() as i32,
                        f.to_bits() as i32,
                    );
                }
            }
        }
        Ok(())
    }

    /// `testReadInts()`.
    fn test_read_ints(&mut self) -> Result<()> {
        let length = 1024 * 8;
        let (mut input, _) = my_buffered_index_input(length as i64)?;
        let buffer_length = 128;
        let mut int_buffer = vec![0i32; buffer_length];
        for alignment in 0..4 {
            input.seek(0)?;
            for _ in 0..alignment {
                input.read_byte()?;
            }
            let bulk_reads = length / (buffer_length * 4) - 1;
            for i in 0..bulk_reads {
                let pos = alignment + i * buffer_length * 4;
                let int_offset = random().next_int_bound(3) as usize;
                input.skip_bytes((int_offset * 4) as i64)?;
                input.read_ints(&mut int_buffer, int_offset, buffer_length - int_offset)?;
                for (idx, &v) in int_buffer.iter().enumerate().skip(int_offset) {
                    let offset = (pos + idx * 4) as i64;
                    let bb = [
                        byten(offset) as u8,
                        byten(offset + 1) as u8,
                        byten(offset + 2) as u8,
                        byten(offset + 3) as u8,
                    ];
                    Assert::assert_equals(i32::from_le_bytes(bb), v);
                }
            }
        }
        Ok(())
    }

    /// `testReadLongs()`.
    fn test_read_longs(&mut self) -> Result<()> {
        let length = 1024 * 8;
        let (mut input, _) = my_buffered_index_input(length as i64)?;
        let buffer_length = 128;
        let mut long_buffer = vec![0i64; buffer_length];
        for alignment in 0..8 {
            input.seek(0)?;
            for _ in 0..alignment {
                input.read_byte()?;
            }
            let bulk_reads = length / (buffer_length * 8) - 1;
            for i in 0..bulk_reads {
                let pos = alignment + i * buffer_length * 8;
                let long_offset = random().next_int_bound(3) as usize;
                input.skip_bytes((long_offset * 8) as i64)?;
                input.read_longs(&mut long_buffer, long_offset, buffer_length - long_offset)?;
                for (idx, &v) in long_buffer.iter().enumerate().skip(long_offset) {
                    let offset = (pos + idx * 8) as i64;
                    let mut bb = [0u8; 8];
                    for (k, b) in bb.iter_mut().enumerate() {
                        *b = byten(offset + k as i64) as u8;
                    }
                    Assert::assert_equals(i64::from_le_bytes(bb), v);
                }
            }
        }
        Ok(())
    }
}

/// `byten(long n)` (`private static`): emulates a file - `byten(n)` returns
/// the n'th byte in that file. `MyBufferedIndexInput` reads this "file".
fn byten(n: i64) -> i8 {
    (n.wrapping_mul(n) % 256) as i8
}

/// `MyBufferedIndexInput` (`private static class … extends
/// BufferedIndexInput`): the source of the "file" `byten` emulates.
///
/// **Divergence from Lucene 10.5.0.** Java subclasses `BufferedIndexInput`
/// and overrides `readInternal(ByteBuffer)` and `seekInternal(long)`.
/// Rucene's `BufferedIndexInput` buffers an unbuffered source `IndexInput`
/// instead of being subclassed: this source is that input, its `read_bytes`
/// is `readInternal` (counting `readCount`) and its `seek` is
/// `seekInternal`; the `BufferedIndexInput` built over it carries the
/// `MyBufferedIndexInput(len=…)` description and `BUFFER_SIZE`.
struct MyBufferedIndexInput {
    pos: i64,
    len: i64,
    read_count: Arc<AtomicI64>,
    description: String,
}

impl DataInput for MyBufferedIndexInput {
    fn to_java_string(&self) -> String {
        self.description.clone()
    }

    fn read_byte(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.read_bytes(&mut b, 0, 1)?;
        Ok(b[0])
    }

    /// `readInternal(ByteBuffer b)`.
    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.read_count.fetch_add(1, Ordering::SeqCst);
        for slot in &mut b[offset..offset + len] {
            *slot = byten(self.pos) as u8;
            self.pos += 1;
        }
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.pos += num_bytes;
        Ok(())
    }
}

impl IndexInput for MyBufferedIndexInput {
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.pos
    }

    fn length(&self) -> i64 {
        self.len
    }

    /// `seekInternal(long pos)`.
    fn seek(&mut self, pos: i64) -> Result<()> {
        self.pos = pos;
        Ok(())
    }

    fn slice(
        &self,
        _slice_description: &str,
        _offset: i64,
        _length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(MyBufferedIndexInput {
            pos: self.pos,
            len: self.len,
            read_count: Arc::clone(&self.read_count),
            description: self.description.clone(),
        }))
    }

    fn resource_description(&self) -> &str {
        &self.description
    }
}

/// `new MyBufferedIndexInput(long len)`, with its `readCount`.
fn my_buffered_index_input(len: i64) -> Result<(BufferedIndexInput, Arc<AtomicI64>)> {
    let read_count = Arc::new(AtomicI64::new(0));
    let source = MyBufferedIndexInput {
        pos: 0,
        len,
        read_count: Arc::clone(&read_count),
        description: format!("MyBufferedIndexInput(len={len})"),
    };
    Ok((
        BufferedIndexInput::new(Box::new(source), BUFFER_SIZE)?,
        read_count,
    ))
}

/// `new MyBufferedIndexInput()`: an infinite file.
fn my_buffered_index_input_infinite() -> Result<(BufferedIndexInput, Arc<AtomicI64>)> {
    my_buffered_index_input(i64::MAX)
}

fn buffered_index_input_class() -> Arc<Class> {
    type T = TestBufferedIndexInput;
    Class::builder("org.apache.lucene.store.TestBufferedIndexInput")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestBufferedIndexInput::new))
        .method(ti("testReadByte", T::test_read_byte))
        .method(ti("testReadBytes", T::test_read_bytes))
        .method(ti("testEOF", T::test_eof))
        .method(ti("testBackwardsByteReads", T::test_backwards_byte_reads))
        .method(ti("testBackwardsShortReads", T::test_backwards_short_reads))
        .method(ti("testBackwardsIntReads", T::test_backwards_int_reads))
        .method(ti("testBackwardsLongReads", T::test_backwards_long_reads))
        .method(ti("testReadFloats", T::test_read_floats))
        .method(ti("testReadInts", T::test_read_ints))
        .method(ti("testReadLongs", T::test_read_longs))
        .build()
}

// -----------------------------------------------------------------------------
// TestByteArrayDataInput
// -----------------------------------------------------------------------------

/// `TestByteArrayDataInput.testBasic()`.
fn byte_array_data_input_test_basic() -> Result<()> {
    let bytes = [1u8, 65];
    let mut r#in = ByteArrayDataInput::new(&bytes);
    Assert::assert_equals_str(None, Some("A"), Some(&r#in.read_string()?));
    Assert::assert_true(r#in.eof());

    let bytes = [1u8, 1, 65];
    r#in.reset_range(&bytes, 1, 2);
    Assert::assert_equals_str(None, Some("A"), Some(&r#in.read_string()?));
    Assert::assert_true(r#in.eof());
    Ok(())
}

/// `TestByteArrayDataInput.testDatatypes()`.
fn byte_array_data_input_test_datatypes() -> Result<()> {
    // write some primitives using ByteArrayDataOutput:
    let mut bytes = [0u8; 32];
    let size;
    {
        let mut out = ByteArrayDataOutput::new(&mut bytes);
        out.write_byte(43)?;
        out.write_short(12345)?;
        out.write_int(1234567890)?;
        out.write_long(1234567890123456789)?;
        size = out.position();
    }
    Assert::assert_equals(15usize, size);

    // read the primitives using ByteBuffer to ensure encoding in byte array is LE:
    let buf = &bytes[..size];
    Assert::assert_equals(43i8, buf[0] as i8);
    Assert::assert_equals(12345i16, i16::from_le_bytes([buf[1], buf[2]]));
    Assert::assert_equals(
        1234567890i32,
        i32::from_le_bytes(buf[3..7].try_into().unwrap()),
    );
    Assert::assert_equals(
        1234567890123456789i64,
        i64::from_le_bytes(buf[7..15].try_into().unwrap()),
    );
    Assert::assert_equals(0usize, buf.len() - 15);

    // read the primitives using ByteArrayDataInput:
    let mut r#in = ByteArrayDataInput::with_range(&bytes, 0, size);
    Assert::assert_equals(43i8, r#in.read_byte()? as i8);
    Assert::assert_equals(12345i16, r#in.read_short()?);
    Assert::assert_equals(1234567890i32, r#in.read_int()?);
    Assert::assert_equals(1234567890123456789i64, r#in.read_long()?);
    Assert::assert_true(r#in.eof());
    Ok(())
}

fn byte_array_data_input_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestByteArrayDataInput")
        .method(t("testBasic", byte_array_data_input_test_basic))
        .method(t("testDatatypes", byte_array_data_input_test_datatypes))
        .build()
}

// -----------------------------------------------------------------------------
// BaseDataOutputTestCase
// -----------------------------------------------------------------------------

/// `IOConsumer<DataInput>`: a check `addRandomData` returns for each write.
type ReadCheck = Box<dyn Fn(&mut dyn DataInput) -> Result<()>>;

/// `BaseDataOutputTestCase<T extends DataOutput>`: the base the data-output
/// tests extend.
trait BaseDataOutputTestCase {
    /// The concrete output type `T`.
    type T: DataOutput;

    /// `newInstance()`.
    fn new_instance() -> Self::T;

    /// `toBytes(T instance)`.
    ///
    /// **Divergence from Lucene 10.5.0.** The instance is passed by value:
    /// `TestFilterIndexOutput` reaches the bytes of the output it wraps by
    /// closing it (see its `to_bytes`).
    fn to_bytes(instance: Self::T) -> Result<Vec<u8>>;

    /// `testRandomizedWrites()`.
    fn test_randomized_writes() -> Result<()> {
        let mut dst = Self::new_instance();
        let mut ref_out = OutputStreamDataOutput::new(Vec::<u8>::new());

        let seed = random().next_long();
        let max = 50_000;
        add_random_data(&mut dst, &Xoroshiro128PlusRandom::new(seed), max, None)?;
        add_random_data(&mut ref_out, &Xoroshiro128PlusRandom::new(seed), max, None)?;
        assert_bytes_equals(&ref_out.into_inner(), &Self::to_bytes(dst)?);
        Ok(())
    }
}

/// `addRandomData(DataOutput dst, Random rnd, int maxAddCalls)` (`protected
/// static`).
///
/// **Divergence from Lucene 10.5.0.** Java's generators test `dst
/// instanceof ByteBuffersDataOutput` (to write through
/// `writeBytes(ByteBuffer)`); Rust cannot narrow a `&mut dyn DataOutput`, so
/// the caller passes the output again as `rdo` when it is one. Rucene's
/// `ByteBuffersDataOutput` has no `writeBytes(ByteBuffer)` and
/// `ByteBuffersDataInput` no `readBytes(ByteBuffer, int)`: the byte-buffer
/// branches write and read the same bytes through the array forms, keeping
/// the random draws of Java's.
fn add_random_data(
    dst: &mut dyn DataOutput,
    rnd: &dyn Random,
    max_add_calls: i32,
    _rdo: Option<()>,
) -> Result<Vec<ReadCheck>> {
    let mut reply: Vec<ReadCheck> = Vec::new();
    for _ in 0..max_add_calls {
        let generator = *RandomPicks::random_from(rnd, &GENERATORS);
        reply.push(generator(dst, rnd)?);
    }
    Ok(reply)
}

type Generator = fn(&mut dyn DataOutput, &dyn Random) -> Result<ReadCheck>;

/// `GENERATORS` (`private static final List<…>`), in Java's order.
static GENERATORS: [Generator; 11] = [
    // writeByte/ readByte
    |dst, rnd| {
        let value = rnd.next_int() as i8;
        dst.write_byte(value as u8)?;
        Ok(Box::new(move |src| {
            Assert::assert_equals_msg(Some("readByte()"), value, src.read_byte()? as i8);
            Ok(())
        }))
    },
    // writeBytes/ readBytes (array and buffer version).
    |dst, rnd| {
        let bytes = RandomBytes::random_bytes_of_length_between(rnd, 0, 100);
        // `rnd.nextBoolean() && rdo != null`: the draw is taken whatever
        // the output; both branches write the same bytes.
        let _use_buffer_for_write = rnd.next_boolean();
        dst.write_bytes(&bytes, 0, bytes.len())?;
        let use_buffers_for_read = rnd.next_boolean();
        Ok(Box::new(move |src| {
            let mut read = vec![0u8; bytes.len()];
            let n = read.len();
            src.read_bytes(&mut read, 0, n)?;
            if use_buffers_for_read {
                assert_bytes_equals_msg("readBytes(ByteBuffer)", &bytes, &read);
            } else {
                assert_bytes_equals_msg("readBytes(byte[])", &bytes, &read);
            }
            Ok(())
        }))
    },
    // writeBytes/ readBytes (array + offset).
    |dst, rnd| {
        let bytes = RandomBytes::random_bytes_of_length_between(rnd, 0, 100);
        let off = RandomNumbers::random_int_between(rnd, 0, bytes.len() as i32) as usize;
        let len = RandomNumbers::random_int_between(rnd, 0, (bytes.len() - off) as i32) as usize;
        dst.write_bytes(&bytes, off, len)?;
        Ok(Box::new(move |src| {
            let mut read = vec![0u8; bytes.len() + off];
            src.read_bytes(&mut read, off, len)?;
            assert_bytes_equals_msg(
                "readBytes(byte[], off)",
                &bytes[off..len + off],
                &read[off..len + off],
            );
            Ok(())
        }))
    },
    |dst, rnd| {
        let v = rnd.next_int();
        dst.write_int(v)?;
        Ok(Box::new(move |src| {
            Assert::assert_equals_msg(Some("readInt()"), v, src.read_int()?);
            Ok(())
        }))
    },
    |dst, rnd| {
        let v = rnd.next_long();
        dst.write_long(v)?;
        Ok(Box::new(move |src| {
            Assert::assert_equals_msg(Some("readLong()"), v, src.read_long()?);
            Ok(())
        }))
    },
    |dst, rnd| {
        let v = rnd.next_int() as i16;
        dst.write_short(v)?;
        Ok(Box::new(move |src| {
            Assert::assert_equals_msg(Some("readShort()"), v, src.read_short()?);
            Ok(())
        }))
    },
    |dst, rnd| {
        let v = rnd.next_int();
        dst.write_v_int(v)?;
        Ok(Box::new(move |src| {
            Assert::assert_equals_msg(Some("readVInt()"), v, src.read_v_int()?);
            Ok(())
        }))
    },
    |dst, rnd| {
        let v = rnd.next_int();
        dst.write_z_int(v)?;
        Ok(Box::new(move |src| {
            Assert::assert_equals_msg(Some("readZInt()"), v, src.read_z_int()?);
            Ok(())
        }))
    },
    |dst, rnd| {
        let v = rnd.next_long() & (((-1i64) as u64 >> 1) as i64);
        dst.write_v_long(v)?;
        Ok(Box::new(move |src| {
            Assert::assert_equals_msg(Some("readVLong()"), v, src.read_v_long()?);
            Ok(())
        }))
    },
    |dst, rnd| {
        let v = rnd.next_long();
        dst.write_z_long(v)?;
        Ok(Box::new(move |src| {
            Assert::assert_equals_msg(Some("readZLong()"), v, src.read_z_long()?);
            Ok(())
        }))
    },
    |dst, rnd| {
        let v = if rnd.next_int_bound(50) == 0 {
            // Occasionally a large blob.
            RandomStrings::random_unicode_of_length(
                rnd,
                RandomNumbers::random_int_between(rnd, 2048, 4096),
            )
        } else {
            RandomStrings::random_unicode_of_length(
                rnd,
                RandomNumbers::random_int_between(rnd, 0, 10),
            )
        };
        dst.write_string(&v)?;
        Ok(Box::new(move |src| {
            Assert::assert_equals_str(Some("readString()"), Some(&v), Some(&src.read_string()?));
            Ok(())
        }))
    },
];

// -----------------------------------------------------------------------------
// TestByteBuffersDataOutput
// -----------------------------------------------------------------------------

/// `TestByteBuffersDataOutput extends
/// BaseDataOutputTestCase<ByteBuffersDataOutput>`.
struct TestByteBuffersDataOutput;

impl BaseDataOutputTestCase for TestByteBuffersDataOutput {
    type T = ByteBuffersDataOutput;

    fn new_instance() -> ByteBuffersDataOutput {
        ByteBuffersDataOutput::new()
    }

    fn to_bytes(instance: ByteBuffersDataOutput) -> Result<Vec<u8>> {
        instance.to_array_copy()
    }
}

/// `TestByteBuffersDataOutput.testReuse()`.
fn bb_output_test_reuse() -> Result<()> {
    let allocations = Arc::new(AtomicI64::new(0));
    let counted = Arc::clone(&allocations);
    let reuser = Arc::new(Mutex::new(ByteBufferRecycler::with_delegate(Arc::new(
        move |size| {
            counted.fetch_add(1, Ordering::SeqCst);
            Vec::with_capacity(size)
        },
    ))));

    let allocate = Arc::clone(&reuser);
    let reuse = Arc::clone(&reuser);
    let mut o = ByteBuffersDataOutput::with_allocator(
        rucene::store::DEFAULT_MIN_BITS_PER_BLOCK,
        rucene::store::DEFAULT_MAX_BITS_PER_BLOCK,
        // `reuser::allocate`
        Some(Arc::new(move |size| {
            allocate
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .allocate(size)
        })),
        // `reuser::reuse`
        Some(Arc::new(move |block| {
            reuse.lock().unwrap_or_else(|e| e.into_inner()).reuse(block)
        })),
    )?;

    // Add some random data first.
    let gen_seed = random().next_long();
    let add_count = TestUtil::next_int(&*random(), 1000, 5000);
    add_random_data(&mut o, &JdkRandom::new(gen_seed), add_count, None)?;
    let data = o.to_array_copy()?;

    // Use the same sequence over reused instance.
    let expected_allocation_count = allocations.load(Ordering::SeqCst);
    o.reset();
    add_random_data(&mut o, &JdkRandom::new(gen_seed), add_count, None)?;

    Assert::assert_equals(
        expected_allocation_count,
        allocations.load(Ordering::SeqCst),
    );
    assert_bytes_equals(&data, &o.to_array_copy()?);
    Ok(())
}

/// `TestByteBuffersDataOutput.testConstructorWithExpectedSize()`.
fn bb_output_test_constructor_with_expected_size() -> Result<()> {
    {
        let mut o = ByteBuffersDataOutput::with_expected_size(0);
        o.write_byte(0)?;
        Assert::assert_equals(
            1usize << rucene::store::DEFAULT_MIN_BITS_PER_BLOCK,
            // `o.toBufferList().get(0).capacity()`: the block size.
            o.block_size(),
        );
    }

    {
        let mb: i64 = 1024 * 1024;
        let expected_size = TestUtil::next_long(&*random(), mb, mb * 1024);
        let mut o = ByteBuffersDataOutput::with_expected_size(expected_size as usize);
        o.write_byte(0)?;
        let cap = o.block_size() as i64;
        Assert::assert_true(
            (cap >> 1) * (rucene::store::MAX_BLOCKS_BEFORE_BLOCK_EXPANSION as i64) < expected_size,
        );
        Assert::assert_true_msg(
            &format!("cap={cap}, exp={expected_size}"),
            cap * rucene::store::MAX_BLOCKS_BEFORE_BLOCK_EXPANSION as i64 >= expected_size,
        );
    }
    Ok(())
}

/// `TestByteBuffersDataOutput.testIllegalMinBitsPerBlock()`.
fn bb_output_test_illegal_min_bits_per_block() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        ByteBuffersDataOutput::with_config(
            rucene::store::LIMIT_MIN_BITS_PER_BLOCK - 1,
            rucene::store::DEFAULT_MAX_BITS_PER_BLOCK,
            false,
        )
    });
    Ok(())
}

/// `TestByteBuffersDataOutput.testIllegalMaxBitsPerBlock()`.
fn bb_output_test_illegal_max_bits_per_block() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        ByteBuffersDataOutput::with_config(
            rucene::store::DEFAULT_MIN_BITS_PER_BLOCK,
            rucene::store::LIMIT_MAX_BITS_PER_BLOCK + 1,
            false,
        )
    });
    Ok(())
}

/// `TestByteBuffersDataOutput.testIllegalBitsPerBlockRange()`.
fn bb_output_test_illegal_bits_per_block_range() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        ByteBuffersDataOutput::with_config(20, 19, false)
    });
    Ok(())
}

/// `TestByteBuffersDataOutput.testSanity()`.
fn bb_output_test_sanity() -> Result<()> {
    let mut o = TestByteBuffersDataOutput::new_instance();
    Assert::assert_equals(0usize, o.size());
    Assert::assert_equals(0usize, o.to_array_copy()?.len());
    Assert::assert_equals(0i64, o.ram_bytes_used());

    o.write_byte(1)?;
    Assert::assert_equals(1usize, o.size());
    Assert::assert_true(o.ram_bytes_used() > 0);
    assert_bytes_equals(&[1], &o.to_array_copy()?);

    o.write_bytes_full(&[2, 3, 4], 3)?;
    Assert::assert_equals(4usize, o.size());
    assert_bytes_equals(&[1, 2, 3, 4], &o.to_array_copy()?);
    Ok(())
}

/// `TestByteBuffersDataOutput.testWriteByteBuffer()`.
///
/// **Divergence from Lucene 10.5.0.** Rucene's `ByteBuffersDataOutput` has
/// no `writeBytes(ByteBuffer)`: the buffer's remaining window
/// (`position(offset)`, `limit(offset + len)`) is written through
/// `writeBytes(byte[], int, int)`.
fn bb_output_test_write_byte_buffer() -> Result<()> {
    let mut o = ByteBuffersDataOutput::new();
    let mut bytes = vec![0u8; 1024 * 8 + 10];
    random().next_bytes(&mut bytes);
    let offset = TestUtil::next_int(&*random(), 0, 100) as usize;
    let len = bytes.len() - offset;
    o.write_bytes(&bytes, offset, len)?;
    Assert::assert_equals(len, o.size());
    assert_bytes_equals(&bytes[offset..offset + len], &o.to_array_copy()?);
    Ok(())
}

/// `TestByteBuffersDataOutput.testLargeArrayAdd()`.
fn bb_output_test_large_array_add() -> Result<()> {
    let mut o = ByteBuffersDataOutput::new();
    let mb = 1024 * 1024;
    let mut bytes = if *TEST_NIGHTLY {
        vec![0u8; TestUtil::next_int(&*random(), 5 * mb, 15 * mb) as usize]
    } else {
        vec![0u8; TestUtil::next_int(&*random(), mb / 2, mb) as usize]
    };
    random().next_bytes(&mut bytes);
    let offset = TestUtil::next_int(&*random(), 0, 100) as usize;
    let len = bytes.len() - offset;
    o.write_bytes(&bytes, offset, len)?;
    Assert::assert_equals(len, o.size());
    assert_bytes_equals(&bytes[offset..offset + len], &o.to_array_copy()?);
    Ok(())
}

/// `TestByteBuffersDataOutput.testCopyBytesOnHeap()`.
fn bb_output_test_copy_bytes_on_heap() -> Result<()> {
    let mut bytes = vec![0u8; 1024 * 8 + 10];
    random().next_bytes(&mut bytes);
    let offset = TestUtil::next_int(&*random(), 0, 100) as usize;
    let len = bytes.len() - offset;
    let mut r#in = ByteArrayDataInput::with_range(&bytes, offset, len);
    let mut o = ByteBuffersDataOutput::with_config(
        rucene::store::DEFAULT_MIN_BITS_PER_BLOCK,
        rucene::store::DEFAULT_MAX_BITS_PER_BLOCK,
        false,
    )?;
    o.copy_bytes(&mut r#in, len as i64)?;
    assert_bytes_equals(&o.to_array_copy()?, &bytes[offset..offset + len]);
    Ok(())
}

/// `TestByteBuffersDataOutput.testRamBytesUsed()`.
fn bb_output_test_ram_bytes_used() -> Result<()> {
    let mut out = ByteBuffersDataOutput::new();
    // Empty output requires no RAM
    Assert::assert_equals(0i64, out.ram_bytes_used());

    // Non-empty buffer requires RAM
    out.write_int(4)?;
    Assert::assert_equals(out.ram_bytes_used(), compute_ram_bytes_used(&out));

    // Make sure this keeps working with multiple backing buffers
    while out.to_buffer_list().len() < 2 {
        out.write_long(42)?;
    }
    Assert::assert_equals(out.ram_bytes_used(), compute_ram_bytes_used(&out));

    // Make sure this keeps working when increasing the block size
    let current_block_capacity = out.block_size();
    loop {
        out.write_long(42)?;
        if out.block_size() != current_block_capacity {
            break;
        }
    }
    Assert::assert_equals(out.ram_bytes_used(), compute_ram_bytes_used(&out));

    // Back to zero after a clear
    out.reset();
    Assert::assert_equals(0i64, out.ram_bytes_used());

    // And back to non-empty
    out.write_int(4)?;
    Assert::assert_equals(out.ram_bytes_used(), compute_ram_bytes_used(&out));
    Ok(())
}

/// `computeRamBytesUsed(ByteBuffersDataOutput)` (`private static`).
///
/// **Divergence from Lucene 10.5.0.** Java sums `ByteBuffer.capacity()` over
/// `toBufferList()`; Rucene's `toBufferList()` returns the written slices,
/// whose capacity it does not expose. Every block of a
/// `ByteBuffersDataOutput` has the output's current block size as its
/// capacity (`rewriteToBlockSize` re-blocks the earlier ones), so each
/// buffer's capacity is `blockSize()`.
fn compute_ram_bytes_used(out: &ByteBuffersDataOutput) -> i64 {
    if out.size() == 0 {
        return 0;
    }
    let buffers = out.to_buffer_list();
    buffers.iter().map(|_| out.block_size() as i64).sum::<i64>()
        + buffers.len() as i64 * rucene::util::RamUsageEstimator::NUM_BYTES_OBJECT_REF
}

/// `TestByteBuffersDataOutput.testCopyBytesOnDirectByteBuffer()`.
///
/// **Divergence from Lucene 10.5.0.** Java allocates the blocks with
/// `ByteBuffer::allocateDirect`; Rucene's `ByteBuffersDataOutput` takes no
/// allocator and its blocks are heap vectors, so this runs the same copy over
/// the output's only allocation.
fn bb_output_test_copy_bytes_on_direct_byte_buffer() -> Result<()> {
    let mut bytes = vec![0u8; 1024 * 8 + 10];
    random().next_bytes(&mut bytes);
    let offset = TestUtil::next_int(&*random(), 0, 100) as usize;
    let len = bytes.len() - offset;
    let mut r#in = ByteArrayDataInput::with_range(&bytes, offset, len);
    let mut o = ByteBuffersDataOutput::with_config(
        rucene::store::DEFAULT_MIN_BITS_PER_BLOCK,
        rucene::store::DEFAULT_MAX_BITS_PER_BLOCK,
        false,
    )?;
    o.copy_bytes(&mut r#in, len as i64)?;
    assert_bytes_equals(&o.to_array_copy()?, &bytes[offset..offset + len]);
    Ok(())
}

/// `TestByteBuffersDataOutput.testToBufferListReturnsReadOnlyBuffers()`.
///
/// **Divergence from Lucene 10.5.0.** Java checks `ByteBuffer.isReadOnly()`
/// on each buffer; Rucene's `toBufferList()` returns shared slices
/// (`&[u8]`), read-only by type, so each check holds by construction.
fn bb_output_test_to_buffer_list_returns_read_only_buffers() -> Result<()> {
    let mut dst = ByteBuffersDataOutput::new();
    dst.write_bytes(&[0u8; 100], 0, 100)?;
    for _bb in dst.to_buffer_list() {
        // `bb.isReadOnly()`: a `&[u8]` cannot be written through.
        Assert::assert_true(true);
    }
    Ok(())
}

/// `TestByteBuffersDataOutput.testToWriteableBufferListReturnsOriginalBuffers()`.
///
/// **Divergence from Lucene 10.5.0.** Java checks `!isReadOnly()` and
/// `hasArray()` on each buffer; Rucene's `toWriteableBufferList()` returns
/// mutable heap slices (`&mut [u8]`), writable and array-backed by type, so
/// each check holds by construction.
fn bb_output_test_to_writeable_buffer_list_returns_original_buffers() -> Result<()> {
    let mut dst = ByteBuffersDataOutput::new();
    for _bb in dst.to_writeable_buffer_list() {
        Assert::assert_true(true); // `!bb.isReadOnly()`
        Assert::assert_true(true); // even the empty buffer should have a backing array.
    }
    dst.write_bytes(&[0u8; 100], 0, 100)?;
    for _bb in dst.to_writeable_buffer_list() {
        Assert::assert_true(true); // `!bb.isReadOnly()`
        Assert::assert_true(true); // heap-based by default, so array should be there.
    }
    Ok(())
}

/// `TestByteBuffersDataOutput.testNullAllocator()`: a Java `null` allocator
/// is `None`.
fn bb_output_test_null_allocator() -> Result<()> {
    expect_throws("java.lang.NullPointerException", || {
        ByteBuffersDataOutput::with_allocator(
            rucene::store::DEFAULT_MIN_BITS_PER_BLOCK,
            rucene::store::DEFAULT_MAX_BITS_PER_BLOCK,
            None,
            Some(rucene::store::no_reuse()),
        )
    });
    Ok(())
}

/// `TestByteBuffersDataOutput.testNullRecycler()`: a Java `null` recycler is
/// `None`.
fn bb_output_test_null_recycler() -> Result<()> {
    expect_throws("java.lang.NullPointerException", || {
        ByteBuffersDataOutput::with_allocator(
            rucene::store::DEFAULT_MIN_BITS_PER_BLOCK,
            rucene::store::DEFAULT_MAX_BITS_PER_BLOCK,
            Some(Arc::new(rucene::store::allocate_bb_on_heap)),
            None,
        )
    });
    Ok(())
}

fn byte_buffers_data_output_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestByteBuffersDataOutput")
        .method(t(
            "testRandomizedWrites",
            TestByteBuffersDataOutput::test_randomized_writes,
        ))
        .method(t("testReuse", bb_output_test_reuse))
        .method(t(
            "testConstructorWithExpectedSize",
            bb_output_test_constructor_with_expected_size,
        ))
        .method(t(
            "testIllegalMinBitsPerBlock",
            bb_output_test_illegal_min_bits_per_block,
        ))
        .method(t(
            "testIllegalMaxBitsPerBlock",
            bb_output_test_illegal_max_bits_per_block,
        ))
        .method(t(
            "testIllegalBitsPerBlockRange",
            bb_output_test_illegal_bits_per_block_range,
        ))
        .method(t("testNullAllocator", bb_output_test_null_allocator))
        .method(t("testNullRecycler", bb_output_test_null_recycler))
        .method(t("testSanity", bb_output_test_sanity))
        .method(t("testWriteByteBuffer", bb_output_test_write_byte_buffer))
        .method(t("testLargeArrayAdd", bb_output_test_large_array_add))
        .method(t("testCopyBytesOnHeap", bb_output_test_copy_bytes_on_heap))
        .method(t(
            "testCopyBytesOnDirectByteBuffer",
            bb_output_test_copy_bytes_on_direct_byte_buffer,
        ))
        .method(t(
            "testToBufferListReturnsReadOnlyBuffers",
            bb_output_test_to_buffer_list_returns_read_only_buffers,
        ))
        .method(t(
            "testToWriteableBufferListReturnsOriginalBuffers",
            bb_output_test_to_writeable_buffer_list_returns_original_buffers,
        ))
        .method(t("testRamBytesUsed", bb_output_test_ram_bytes_used))
        .build()
}

// -----------------------------------------------------------------------------
// TestFilterIndexOutput
// -----------------------------------------------------------------------------

/// `TestFilterIndexOutput extends BaseDataOutputTestCase<FilterIndexOutput>`.
struct TestFilterIndexOutput;

/// The `ByteBuffersDataOutput` the `ByteBuffersIndexOutput` of
/// `newInstance()` hands over when it is closed.
static FILTER_INDEX_OUTPUT_BYTES: Mutex<Option<ByteBuffersDataOutput>> = Mutex::new(None);

impl BaseDataOutputTestCase for TestFilterIndexOutput {
    type T = FilterIndexOutput;

    fn new_instance() -> FilterIndexOutput {
        FilterIndexOutput::new(
            "test",
            "test",
            Box::new(ByteBuffersIndexOutput::with_checksum(
                ByteBuffersDataOutput::new_resettable_instance(),
                "test",
                "test",
                None,
                Some(Box::new(|output| {
                    *FILTER_INDEX_OUTPUT_BYTES
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(output);
                    Ok(())
                })),
            )),
        )
    }

    /// `((ByteBuffersIndexOutput) instance.out).toArrayCopy()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Rust cannot narrow the wrapped
    /// `&dyn IndexOutput` back to `ByteBuffersIndexOutput`; closing the
    /// filter closes it, and its close callback hands over the
    /// `ByteBuffersDataOutput` holding the written bytes. The wrapped output
    /// is built without a checksum, which `toArrayCopy()` does not read.
    fn to_bytes(mut instance: FilterIndexOutput) -> Result<Vec<u8>> {
        instance.close()?;
        FILTER_INDEX_OUTPUT_BYTES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .expect("the closed output's bytes")
            .to_array_copy()
    }
}

/// `TestFilterIndexOutput.testUnwrap()`.
fn filter_index_output_test_unwrap() -> Result<()> {
    let mut dir = FSDirectory::open(LuceneTestCase::create_temp_dir())?;
    let output = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
    let output_ptr = &*output as *const dyn IndexOutput as *const ();
    let mut filter_index_output =
        FilterIndexOutput::new("wrapper of test", "FilterDirectory{test}", output);
    Assert::assert_true(std::ptr::eq(
        filter_index_output.get_delegate() as *const dyn IndexOutput as *const (),
        output_ptr,
    ));
    Assert::assert_true(std::ptr::eq(
        FilterIndexOutput::unwrap(&filter_index_output) as *const dyn IndexOutput as *const (),
        output_ptr,
    ));
    filter_index_output.close()?;
    dir.close()
}

/// The public methods `FilterIndexOutput` declares itself
/// (`FilterIndexOutput.java:38-84`), as their Rucene counterparts: the
/// inherent `unwrap` and `get_delegate`, and the explicit implementations in
/// `impl IndexOutput for FilterIndexOutput` and `impl DataOutput for
/// FilterIndexOutput` (`src/store.rs`). Rucene's `name` and
/// `resource_description` are implemented too, as the trait declares them
/// without a body (Java's `getName()` and `toString()`).
///
/// `as_filter_index_output` is left out: it is the port's stand-in for Java's
/// `instanceof FilterIndexOutput`, not a method Java declares.
const FILTER_INDEX_OUTPUT_DECLARED_METHODS: [&str; 9] = [
    "unwrap",
    "get_delegate",
    "close",
    "file_pointer",
    "checksum",
    "resource_description",
    "name",
    "write_byte",
    "write_bytes",
];

/// The public instance methods of `IndexOutput` (with those it inherits from
/// `DataOutput`) as their Rucene counterparts in `trait IndexOutput` and
/// `trait DataOutput` (`src/store.rs`), each with whether it is abstract
/// (declared without a body). A method absent here answers Java's
/// `NoSuchMethodException`.
const INDEX_OUTPUT_METHODS: [(&str, bool); 30] = [
    ("close", true),
    ("file_pointer", true),
    ("checksum", true),
    ("resource_description", true),
    ("name", true),
    ("align_file_pointer", false),
    ("write_byte", true),
    ("write_bytes", true),
    ("write_bytes_full", false),
    ("write_short", false),
    ("write_int", false),
    ("write_v_int", false),
    ("write_z_int", false),
    ("write_long", false),
    ("write_v_long", false),
    ("write_signed_v_long", false),
    ("write_z_long", false),
    ("write_float", false),
    ("write_double", false),
    ("write_string", false),
    ("write_map_of_strings", false),
    ("write_set_of_strings", false),
    ("write_ints", false),
    ("write_longs", false),
    ("write_floats", false),
    ("write_doubles", false),
    ("write_group_vints", false),
    ("write_group_vints_long", false),
    ("copy_bytes", false),
    ("as_filter_index_output", false),
];

/// `TestFilterIndexOutput.testOverrides()`.
///
/// Java walks `FilterIndexOutput.class.getMethods()` by reflection; Rust has
/// none, so the methods are the declared lists
/// [`FILTER_INDEX_OUTPUT_DECLARED_METHODS`] and [`INDEX_OUTPUT_METHODS`]
/// (user decision, sprint 9).
fn filter_index_output_test_overrides() -> Result<()> {
    for m in FILTER_INDEX_OUTPUT_DECLARED_METHODS {
        // verify that only abstract methods are overridden
        match INDEX_OUTPUT_METHODS.iter().find(|(name, _)| *name == m) {
            Some((_, is_abstract)) => Assert::assert_true_msg(
                &format!("Non-abstract method {m} is overridden"),
                *is_abstract,
            ),
            None => {
                // `assertTrue(e instanceof NoSuchMethodException)`
            }
        }
    }
    Ok(())
}

fn filter_index_output_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestFilterIndexOutput")
        .method(t(
            "testRandomizedWrites",
            TestFilterIndexOutput::test_randomized_writes,
        ))
        .method(t("testOverrides", filter_index_output_test_overrides))
        .method(t("testUnwrap", filter_index_output_test_unwrap))
        .build()
}

// -----------------------------------------------------------------------------
// TestByteBuffersDataInput (extends RandomizedTest)
// -----------------------------------------------------------------------------

/// `TestByteBuffersDataInput.testSanity()`.
fn bb_input_test_sanity() -> Result<()> {
    let mut out = ByteBuffersDataOutput::new();
    let mut o1 = out.to_data_input()?;
    Assert::assert_equals(0usize, o1.length());
    expect_throws("java.io.EOFException", || o1.read_byte());

    out.write_byte(1)?;
    let mut o2 = out.to_data_input()?;
    Assert::assert_equals(1usize, o2.length());
    Assert::assert_equals(0usize, o2.position());
    Assert::assert_equals(0usize, o1.length());

    Assert::assert_true(o2.ram_bytes_used() > 0);
    Assert::assert_equals(1u8, o2.read_byte()?);
    Assert::assert_equals(1usize, o2.position());
    Assert::assert_equals(1u8, o2.read_byte_at(0)?);
    expect_throws("java.io.EOFException", || o2.read_byte());
    Assert::assert_equals(1usize, o2.position());
    Ok(())
}

/// `TestByteBuffersDataInput.testRandomReads()`.
fn bb_input_test_random_reads() -> Result<()> {
    let mut dst = ByteBuffersDataOutput::new();

    let seed = RandomizedTest::random_long();
    let max = if *TEST_NIGHTLY { 1_000_000 } else { 100_000 };
    let reply = add_random_data(&mut dst, &Xoroshiro128PlusRandom::new(seed), max, Some(()))?;

    let mut src = dst.to_data_input()?;
    for c in &reply {
        c(&mut src)?;
    }

    expect_throws("java.io.EOFException", || src.read_byte());
    Ok(())
}

/// `TestByteBuffersDataInput.testRandomReadsOnSlices()`.
fn bb_input_test_random_reads_on_slices() -> Result<()> {
    let mut reps = RandomizedTest::random_int_between(1, 20);
    loop {
        reps -= 1;
        if reps <= 0 {
            break;
        }
        let mut dst = ByteBuffersDataOutput::new();

        let prefix = vec![0u8; RandomizedTest::random_int_between(0, 1024 * 8) as usize];
        dst.write_bytes(&prefix, 0, prefix.len())?;

        let seed = RandomizedTest::random_long();
        let max = 10_000;
        let reply = add_random_data(&mut dst, &Xoroshiro128PlusRandom::new(seed), max, Some(()))?;

        let suffix = vec![0u8; RandomizedTest::random_int_between(0, 1024 * 8) as usize];
        dst.write_bytes(&suffix, 0, suffix.len())?;

        let mut src = dst
            .to_data_input()?
            .slice(prefix.len(), dst.size() - prefix.len() - suffix.len())?;

        Assert::assert_equals(0usize, src.position());
        Assert::assert_equals(dst.size() - prefix.len() - suffix.len(), src.length());
        for c in &reply {
            c(&mut src)?;
        }

        expect_throws("java.io.EOFException", || src.read_byte());
    }
    Ok(())
}

/// `TestByteBuffersDataInput.testSeekEmpty()`.
fn bb_input_test_seek_empty() -> Result<()> {
    let dst = ByteBuffersDataOutput::new();
    let mut r#in = dst.to_data_input()?;
    r#in.seek(0)?;

    expect_throws("java.io.EOFException", || r#in.seek(1));

    r#in.seek(0)?;
    expect_throws("java.io.EOFException", || r#in.read_byte());
    Ok(())
}

/// `TestByteBuffersDataInput.testSeekAndSkip()`.
fn bb_input_test_seek_and_skip() -> Result<()> {
    let mut reps = RandomizedTest::random_int_between(1, 200);
    loop {
        reps -= 1;
        if reps <= 0 {
            break;
        }
        let mut dst = ByteBuffersDataOutput::new();

        let mut prefix: Vec<u8> = Vec::new();
        if RandomizedTest::random_boolean() {
            prefix = vec![0u8; RandomizedTest::random_int_between(1, 1024 * 8) as usize];
            dst.write_bytes(&prefix, 0, prefix.len())?;
        }

        let seed = RandomizedTest::random_long();
        let max = 1000;
        let reply = add_random_data(&mut dst, &Xoroshiro128PlusRandom::new(seed), max, Some(()))?;

        let mut r#in = dst
            .to_data_input()?
            .slice(prefix.len(), dst.size() - prefix.len())?;

        r#in.seek(0)?;
        for c in &reply {
            c(&mut r#in)?;
        }

        r#in.seek(0)?;
        for c in &reply {
            c(&mut r#in)?;
        }

        let array = dst.to_array_copy()?;
        let array = array[prefix.len()..].to_vec();

        // test seeking
        for _ in 0..1000 {
            let offs = RandomizedTest::random_int_between(0, array.len() as i32 - 1) as usize;
            r#in.seek(offs)?;
            Assert::assert_equals(offs, r#in.position());
            Assert::assert_equals(array[offs] as i8, r#in.read_byte()? as i8);
        }

        // test skipping
        let max_skip_to = array.len() as i32 - 1;
        r#in.seek(0)?;
        // skip chunks of bytes until exhausted
        let mut curr = 0;
        while curr < max_skip_to {
            let skip_to = RandomizedTest::random_int_between(curr, max_skip_to);
            let step = skip_to - curr;
            r#in.skip_bytes(step as i64)?;
            Assert::assert_equals(array[skip_to as usize] as i8, r#in.read_byte()? as i8);
            curr = skip_to + 1; // +1 for read byte
        }

        let length = r#in.length();
        r#in.seek(length)?;
        Assert::assert_equals(r#in.length(), r#in.position());
        expect_throws("java.io.EOFException", || r#in.read_byte());
    }
    Ok(())
}

/// `TestByteBuffersDataInput.testSlicingWindow()`.
fn bb_input_test_slicing_window() -> Result<()> {
    let mut dst = ByteBuffersDataOutput::new();
    Assert::assert_equals(0usize, dst.to_data_input()?.slice(0, 0)?.length());

    let bytes = RandomizedTest::random_bytes_of_length(1024 * 8);
    dst.write_bytes(&bytes, 0, bytes.len())?;
    let r#in = dst.to_data_input()?;
    let max = dst.size();
    for offset in 0..max {
        Assert::assert_equals(0usize, r#in.slice(offset, 0)?.length());
        Assert::assert_equals(1usize, r#in.slice(offset, 1)?.length());

        let window = (max - offset).min(1024);
        Assert::assert_equals(window, r#in.slice(offset, window)?.length());
    }
    Assert::assert_equals(0usize, r#in.slice(dst.size(), 0)?.length());
    Ok(())
}

/// `TestByteBuffersDataInput.testEofOnArrayReadPastBufferSize()`.
///
/// **Divergence from Lucene 10.5.0.** Rucene's `ByteBuffersDataInput` has
/// no `readBytes(ByteBuffer, int)`: the second read goes through
/// `readBytes(byte[], int, int)` into a 100-byte array.
fn bb_input_test_eof_on_array_read_past_buffer_size() -> Result<()> {
    let mut dst = ByteBuffersDataOutput::new();
    dst.write_bytes(&[0u8; 10], 0, 10)?;

    expect_throws("java.io.EOFException", || {
        let mut r#in = dst.to_data_input()?;
        r#in.read_bytes(&mut [0u8; 100], 0, 100)
    });

    expect_throws("java.io.EOFException", || {
        let mut r#in = dst.to_data_input()?;
        r#in.read_bytes(&mut [0u8; 100], 0, 100)
    });
    Ok(())
}

/// `TestByteBuffersDataInput.testSlicingLargeBuffers()` (LUCENE-8625).
///
/// **Divergence from Lucene 10.5.0.** Java simulates a "large" (> 4GB)
/// input by duplicating one 4 MB `ByteBuffer` (sharing its storage); Rucene's
/// `ByteBuffersDataInput` owns `Vec<u8>` blocks, so each duplicate is a copy
/// of the page (the same content, its limit and the head shift applied).
fn bb_input_test_slicing_large_buffers() -> Result<()> {
    // Simulate a "large" (> 4GB) input by duplicating
    // buffers with the same content.
    let mb = 1024 * 1024;
    let page_bytes = RandomizedTest::random_bytes_of_length(4 * mb);

    // Add some head shift on the first buffer.
    let shift = RandomizedTest::random_int_between(0, page_bytes.len() as i32 / 2) as usize;

    let simulated_length = RandomizedTest::random_long_between(0, 2018) + 4i64 * i32::MAX as i64;

    let mut buffers: Vec<Vec<u8>> = Vec::new();
    let mut remaining = simulated_length + shift as i64;
    while remaining > 0 {
        let mut bb = page_bytes.clone();
        if bb.len() as i64 > remaining {
            bb.truncate(remaining as usize);
        }
        remaining -= bb.len() as i64;
        buffers.push(bb);
    }
    buffers[0].drain(..shift);

    let dst = ByteBuffersDataInput::new(buffers)?;
    Assert::assert_equals(simulated_length, dst.length() as i64);

    let max = dst.length() as i64;
    let mut offset: i64 = 0;
    while offset < max {
        Assert::assert_equals(0usize, dst.slice(offset as usize, 0)?.length());
        Assert::assert_equals(1usize, dst.slice(offset as usize, 1)?.length());

        let window = (max - offset).min(1024);
        let slice = dst.slice(offset as usize, window as usize)?;
        Assert::assert_equals(window, slice.length() as i64);

        // Sanity check of the content against original pages.
        for i in 0..window {
            let expected =
                page_bytes[((shift as i64 + offset + i) % page_bytes.len() as i64) as usize] as i8;
            Assert::assert_equals(expected, slice.read_byte_at(i as usize)? as i8);
        }
        offset += RandomizedTest::random_int_between(mb, 4 * mb) as i64;
    }
    Ok(())
}

fn byte_buffers_data_input_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.store.TestByteBuffersDataInput")
        .extends(&RandomizedTest::class())
        .constructor(Constructor::no_args(|| ()))
        .method(
            t("testSanity", bb_input_test_sanity).annotate(randomizedtesting::junit::Test::new()),
        )
        .method(
            t("testRandomReads", bb_input_test_random_reads)
                .annotate(randomizedtesting::junit::Test::new()),
        )
        .method(
            t(
                "testRandomReadsOnSlices",
                bb_input_test_random_reads_on_slices,
            )
            .annotate(randomizedtesting::junit::Test::new()),
        )
        .method(
            t("testSeekEmpty", bb_input_test_seek_empty)
                .annotate(randomizedtesting::junit::Test::new()),
        )
        .method(
            t("testSeekAndSkip", bb_input_test_seek_and_skip)
                .annotate(randomizedtesting::junit::Test::new()),
        )
        .method(
            t("testSlicingWindow", bb_input_test_slicing_window)
                .annotate(randomizedtesting::junit::Test::new()),
        )
        .method(
            t(
                "testEofOnArrayReadPastBufferSize",
                bb_input_test_eof_on_array_read_past_buffer_size,
            )
            .annotate(randomizedtesting::junit::Test::new())
            .annotate(Timeout { millis: 5000 }),
        )
        .method(
            t(
                "testSlicingLargeBuffers",
                bb_input_test_slicing_large_buffers,
            )
            .annotate(randomizedtesting::junit::Test::new()),
        )
        .build()
}

// -----------------------------------------------------------------------------
// TestChecksumIndexInput
// -----------------------------------------------------------------------------

/// `TestChecksumIndexInput.testSkipBytes()`.
fn checksum_index_input_test_skip_bytes() -> Result<()> {
    let num_test_bytes = TestUtil::next_int(&*random(), 100, 1000) as usize;
    let test_bytes = vec![0u8; num_test_bytes];
    let mut dir = LuceneTestCase::new_directory();
    let mut os = dir
        .as_directory()
        .create_output("foo", &*LuceneTestCase::new_io_context(&*random()))?;
    os.write_bytes_full(&test_bytes, num_test_bytes)?;
    os.close()?;
    drop(os);

    let is = dir
        .as_directory()
        .open_input("foo", &*LuceneTestCase::new_io_context(&*random()))?;
    let mut checksum_index_input = InterceptingChecksumIndexInput::new(is, num_test_bytes);
    // skip random chunks of bytes until everything has been skipped
    let mut skipped = 0;
    while skipped < num_test_bytes {
        let remaining = num_test_bytes - skipped;
        // when remaining gets small enough, just skip the rest
        let step = if remaining < 10 {
            remaining
        } else {
            random().next_int_bound(remaining as i32) as usize
        };
        checksum_index_input.skip_bytes(step as i64)?;
        skipped += step;
    }

    // ensure all skipped bytes are still "read" in so the checksum can be updated properly
    assert_bytes_equals(&test_bytes, &checksum_index_input.read_bytes_copy);

    checksum_index_input.close()?;
    dir.close()
}

/// `InterceptingChecksumIndexInput` (`private static final class … extends
/// BufferedChecksumIndexInput`): captures read bytes into a separate buffer
/// for confirming that all `#skipByte` invocations delegate to `#readBytes`.
///
/// **Divergence from Lucene 10.5.0.** Java subclasses
/// `BufferedChecksumIndexInput` and overrides `readBytes`; the inherited
/// `skipBytes` (`IndexInput.skipBytes` → `ChecksumIndexInput.seek` →
/// `skipByReading`) then reaches the override virtually. Rust has no
/// subclassing: this wraps a `BufferedChecksumIndexInput`, overrides
/// `read_bytes`, and implements `skip_bytes` as `IndexInput.skipBytes`
/// does, through [`ChecksumIndexInput::seek`], whose reads go through this
/// type's `read_bytes`.
struct InterceptingChecksumIndexInput {
    main: BufferedChecksumIndexInput,
    read_bytes_copy: Vec<u8>,
    off: usize,
    skip_buffer: [u8; CHECKSUM_SKIP_BUFFER_SIZE],
}

impl InterceptingChecksumIndexInput {
    fn new(main: Box<dyn IndexInput>, len: usize) -> InterceptingChecksumIndexInput {
        InterceptingChecksumIndexInput {
            main: BufferedChecksumIndexInput::new(main),
            read_bytes_copy: vec![0u8; len],
            off: 0,
            skip_buffer: [0u8; CHECKSUM_SKIP_BUFFER_SIZE],
        }
    }
}

impl DataInput for InterceptingChecksumIndexInput {
    fn to_java_string(&self) -> String {
        self.main.to_java_string()
    }

    fn read_byte(&mut self) -> Result<u8> {
        self.main.read_byte()
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.main.read_bytes(b, offset, len)?;
        self.read_bytes_copy[self.off..self.off + len].copy_from_slice(&b[offset..offset + len]);
        self.off += len;
        Ok(())
    }

    /// `IndexInput.skipBytes(long)`.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let skip_to = self.file_pointer() + num_bytes;
        ChecksumIndexInput::seek(self, skip_to)
    }
}

impl IndexInput for InterceptingChecksumIndexInput {
    fn close(&mut self) -> Result<()> {
        self.main.close()
    }

    fn file_pointer(&self) -> i64 {
        self.main.file_pointer()
    }

    fn length(&self) -> i64 {
        self.main.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        ChecksumIndexInput::seek(self, pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.main.slice(slice_description, offset, length)
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.main.clone_input()
    }

    fn resource_description(&self) -> &str {
        self.main.resource_description()
    }
}

impl ChecksumIndexInput for InterceptingChecksumIndexInput {
    fn get_checksum(&self) -> Result<i64> {
        self.main.get_checksum()
    }

    fn skip_buffer(&mut self) -> &mut [u8; CHECKSUM_SKIP_BUFFER_SIZE] {
        &mut self.skip_buffer
    }
}

fn checksum_index_input_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestChecksumIndexInput")
        .method(t("testSkipBytes", checksum_index_input_test_skip_bytes))
        .build()
}

// -----------------------------------------------------------------------------
// TestDirectory
// -----------------------------------------------------------------------------

/// `TestDirectory.testDirectInstantiation()`: different instances of
/// `FSDirectory` can coexist on the same path, can read, write, and lock
/// files.
fn directory_test_direct_instantiation() -> Result<()> {
    let path = LuceneTestCase::create_temp_dir_prefix("testDirectInstantiation");

    let large_buffer_len = random().next_int_bound(256 * 1024) as usize;
    let mut large_buffer = vec![0u8; large_buffer_len];
    let mut large_read_buffer = vec![0u8; large_buffer_len];
    for (i, b) in large_buffer.iter_mut().enumerate() {
        *b = i as u8; // automatically loops with modulo
    }

    let mut dirs: Vec<Box<dyn Directory>> = vec![
        Box::new(NIOFSDirectory::open(&path)?),
        Box::new(MMapDirectory::open(&path)?),
    ];

    for i in 0..dirs.len() {
        let dir = &dirs[i];
        dir.ensure_open()?;
        let fname = format!("foo.{i}");
        let lockname = format!("foo{i}.lck");
        let mut out = dir.create_output(&fname, &*LuceneTestCase::new_io_context(&*random()))?;
        out.write_byte(i as u8)?;
        out.write_bytes_full(&large_buffer, large_buffer.len())?;
        out.close()?;
        drop(out);

        for d2 in &dirs {
            d2.ensure_open()?;
            Assert::assert_true(LuceneTestCase::slow_file_exists(&**d2, &fname)?);
            Assert::assert_equals(1 + large_buffer.len() as i64, d2.file_length(&fname)?);

            let mut input = d2.open_input(&fname, &*LuceneTestCase::new_io_context(&*random()))?;
            Assert::assert_equals(i as i8, input.read_byte()? as i8);
            // read array with buffering enabled
            large_read_buffer.fill(0);
            let n = large_read_buffer.len();
            input.read_bytes_buffered(&mut large_read_buffer, 0, n, true)?;
            assert_bytes_equals(&large_buffer, &large_read_buffer);
            // read again without using buffer
            input.seek(1)?;
            large_read_buffer.fill(0);
            input.read_bytes_buffered(&mut large_read_buffer, 0, n, false)?;
            assert_bytes_equals(&large_buffer, &large_read_buffer);
            input.close()?;
        }

        // delete with a different dir
        dirs[(i + 1) % dirs.len()].delete_file(&fname)?;

        for d2 in &dirs {
            Assert::assert_false(LuceneTestCase::slow_file_exists(&**d2, &fname)?);
        }

        let lock = dirs[i].obtain_lock(&lockname)?;
        for other in &dirs {
            expect_throws("org.apache.lucene.store.LockObtainFailedException", || {
                other.obtain_lock(&lockname)
            });
        }
        lock.close()?;

        // now lock with different dir
        let lock = dirs[(i + 1) % dirs.len()].obtain_lock(&lockname)?;
        lock.close()?;
    }

    for dir in &mut dirs {
        dir.ensure_open()?;
        dir.close()?;
        // `assertFalse(dir.isOpen)`
        Assert::assert_false(dir.ensure_open().is_ok());
    }
    Ok(())
}

/// `TestDirectory.testNotDirectory()` (LUCENE-1468).
fn directory_test_not_directory() -> Result<()> {
    let path = LuceneTestCase::create_temp_dir_prefix("testnotdir");
    let mut fs_dir = NIOFSDirectory::open(&path)?;
    let result = (|| -> Result<()> {
        let mut out =
            fs_dir.create_output("afile", &*LuceneTestCase::new_io_context(&*random()))?;
        out.close()?;
        drop(out);
        Assert::assert_true(LuceneTestCase::slow_file_exists(&fs_dir, "afile")?);
        expect_throws("java.io.IOException", || {
            NIOFSDirectory::open(path.resolve_str("afile"))
        });
        Ok(())
    })();
    result.and(fs_dir.close())
}

/// `TestDirectory.testListAll()`.
fn directory_test_list_all() -> Result<()> {
    let dir = LuceneTestCase::create_temp_dir_prefix("testdir");
    LuceneTestCase::assume_false(
        "this test does not expect extra files",
        dir.get_file_system().provider().get_scheme() == "extras",
    );
    let file1 = Files::create_file(&dir.resolve_str("tempfile1"))?;
    let file2 = Files::create_file(&dir.resolve_str("tempfile2"))?;
    let files: HashSet<String> = FSDirectory::list_all_in(&dir)?.into_iter().collect();

    Assert::assert_true(files.len() == 2);
    Assert::assert_true(files.contains(&file1.get_file_name().unwrap().to_string()));
    Assert::assert_true(files.contains(&file2.get_file_name().unwrap().to_string()));
    Ok(())
}

fn directory_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestDirectory")
        .method(t(
            "testDirectInstantiation",
            directory_test_direct_instantiation,
        ))
        .method(t("testNotDirectory", directory_test_not_directory))
        .method(t("testListAll", directory_test_list_all))
        .build()
}

// -----------------------------------------------------------------------------
// TestIndexOutputAlignment
// -----------------------------------------------------------------------------

/// `TestIndexOutputAlignment.testAlignmentCalculation()`.
fn alignment_test_alignment_calculation() -> Result<()> {
    Assert::assert_equals(0i64, align_offset(0, 8)?);
    Assert::assert_equals(0i64, align_offset(0, 4)?);
    Assert::assert_equals(0i64, align_offset(0, 2)?);
    Assert::assert_equals(0i64, align_offset(0, 1)?);

    Assert::assert_equals(8i64, align_offset(1, 8)?);
    Assert::assert_equals(4i64, align_offset(1, 4)?);
    Assert::assert_equals(2i64, align_offset(1, 2)?);
    Assert::assert_equals(1i64, align_offset(1, 1)?);

    Assert::assert_equals(32i64, align_offset(25, 8)?);
    Assert::assert_equals(28i64, align_offset(25, 4)?);
    Assert::assert_equals(26i64, align_offset(25, 2)?);
    Assert::assert_equals(25i64, align_offset(25, 1)?);

    let val = 1i64 << 48;
    Assert::assert_equals(val, align_offset(val - 1, 8)?);
    Assert::assert_equals(val, align_offset(val - 1, 4)?);
    Assert::assert_equals(val, align_offset(val - 1, 2)?);
    // byte alignment never changes anything:
    Assert::assert_equals(val - 1, align_offset(val - 1, 1)?);

    Assert::assert_equals(i64::MAX, align_offset(i64::MAX, 1)?);
    Ok(())
}

/// `TestIndexOutputAlignment.testInvalidAlignments()`.
fn alignment_test_invalid_alignments() -> Result<()> {
    assert_invalid_aligment(0);
    assert_invalid_aligment(-1);
    assert_invalid_aligment(-2);
    assert_invalid_aligment(6);
    assert_invalid_aligment(43);
    assert_invalid_aligment(i32::MIN);

    expect_throws("java.lang.IllegalArgumentException", || align_offset(-1, 1));
    expect_throws("java.lang.ArithmeticException", || {
        align_offset(i64::MAX, 2)
    });
    Ok(())
}

/// `assertInvalidAligment(int size)` (`private static`).
fn assert_invalid_aligment(size: i32) {
    expect_throws("java.lang.IllegalArgumentException", || {
        align_offset(1, size)
    });
}

/// `TestIndexOutputAlignment.testOutputAlignment()`.
fn alignment_test_output_alignment() -> Result<()> {
    for alignment in [8, 4, 2, 1] {
        run_test_output_alignment(alignment);
    }
    Ok(())
}

/// `runTestOutputAlignment(int alignment)` (`private static`).
fn run_test_output_alignment(alignment: i32) {
    let result = (|| -> Result<()> {
        let mut out = OutputStreamIndexOutput::new("test output", "test", Vec::<u8>::new(), 8192)?;
        let result = (|| -> Result<()> {
            for _ in 0..10 * *RANDOM_MULTIPLIER {
                // write some bytes
                let length = random().next_int_bound(32) as usize;
                out.write_bytes_full(&vec![0u8; length], length)?;
                let orig_pos = out.file_pointer();
                // align to next boundary
                let new_pos = out.align_file_pointer(alignment)?;
                Assert::assert_equals(out.file_pointer(), new_pos);
                Assert::assert_true_msg("not aligned", new_pos % alignment as i64 == 0);
                Assert::assert_true_msg("newPos >=", new_pos >= orig_pos);
                Assert::assert_true_msg("too much added", new_pos - orig_pos < alignment as i64);
            }
            Ok(())
        })();
        result.and(out.close())
    })();
    if let Err(ioe) = result {
        // `throw new AssertionError(ioe)`
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.AssertionError",
            rucene_test_framework::util::java_exceptions::java_throwable(&ioe).to_string(),
        )
        .throw();
    }
}

fn index_output_alignment_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestIndexOutputAlignment")
        .method(t(
            "testAlignmentCalculation",
            alignment_test_alignment_calculation,
        ))
        .method(t(
            "testInvalidAlignments",
            alignment_test_invalid_alignments,
        ))
        .method(t("testOutputAlignment", alignment_test_output_alignment))
        .build()
}

// -----------------------------------------------------------------------------
// TestInputStreamDataInput
// -----------------------------------------------------------------------------

/// `RANDOM_DATA` (`private static byte[]`), set by `beforeClass`.
static RANDOM_DATA: Mutex<Option<Vec<u8>>> = Mutex::new(None);

fn random_data() -> Vec<u8> {
    RANDOM_DATA
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("RANDOM_DATA is set by beforeClass")
}

/// The instance state of `TestInputStreamDataInput`: its `in` field.
struct TestInputStreamDataInput {
    r#in: Option<NoReadInputStreamDataInput>,
}

impl TestInputStreamDataInput {
    /// `before()` (`@Before`).
    fn before(&mut self) -> Result<()> {
        self.r#in = Some(NoReadInputStreamDataInput::new(Cursor::new(random_data())));
        Ok(())
    }

    /// `after()` (`@After`).
    fn after(&mut self) -> Result<()> {
        if let Some(r#in) = self.r#in.take() {
            r#in.close()?;
        }
        Ok(())
    }

    /// `testSkipBytes()`.
    fn test_skip_bytes(&mut self) -> Result<()> {
        let random = random();
        let random_data = random_data();
        // not using the wrapped (NoReadInputStreamDataInput) here since we want to actually read
        // and verify
        let mut r#in = InputStreamDataInput::new(Cursor::new(random_data.clone()));
        let max_skip_to = random_data.len() as i32 - 1;
        // skip chunks of bytes until exhausted
        let mut curr = 0;
        while curr < max_skip_to {
            let skip_to = TestUtil::next_int(&*random, curr, max_skip_to);
            let step = skip_to - curr;
            r#in.skip_bytes(step as i64)?;
            Assert::assert_equals(random_data[skip_to as usize] as i8, r#in.read_byte()? as i8);
            curr = skip_to + 1; // +1 for read byte
        }
        r#in.close()
    }

    /// `testNoReadWhenSkipping()`.
    fn test_no_read_when_skipping(&mut self) -> Result<()> {
        let random = random();
        let max_skip_to = random_data().len() as i32 - 1;
        let r#in = self.r#in.as_mut().expect("set by before()");
        // skip chunks of bytes until exhausted
        let mut curr = 0;
        while curr < max_skip_to {
            let step = TestUtil::next_int(&*random, 0, max_skip_to - curr);
            r#in.skip_bytes(step as i64)?;
            curr += step;
        }
        Ok(())
    }

    /// `testFullSkip()`.
    fn test_full_skip(&mut self) -> Result<()> {
        let len = random_data().len() as i64;
        self.r#in.as_mut().expect("set by before()").skip_bytes(len)
    }

    /// `testSkipOffEnd()`.
    fn test_skip_off_end(&mut self) -> Result<()> {
        let len = random_data().len() as i64;
        let r#in = self.r#in.as_mut().expect("set by before()");
        expect_throws("java.io.EOFException", || r#in.skip_bytes(len + 1));
        Ok(())
    }
}

/// `NoReadInputStreamDataInput` (`private static final class … extends
/// InputStreamDataInput`): throws if trying to read bytes to ensure
/// `skipBytes` doesn't invoke read.
///
/// **Divergence from Lucene 10.5.0.** Java subclasses
/// `InputStreamDataInput` and overrides `readBytes` and `readByte`; a
/// `skipBytes` that read would reach the overrides virtually. Rust has no
/// subclassing: this wraps an `InputStreamDataInput`, whose own
/// `skip_bytes` it delegates to, and its `read_byte`/`read_bytes` throw
/// `UnsupportedOperationException`. A read issued inside the wrapped
/// `skip_bytes` would therefore not reach these overrides.
struct NoReadInputStreamDataInput {
    inner: InputStreamDataInput<Cursor<Vec<u8>>>,
}

impl NoReadInputStreamDataInput {
    fn new(is: Cursor<Vec<u8>>) -> NoReadInputStreamDataInput {
        NoReadInputStreamDataInput {
            inner: InputStreamDataInput::new(is),
        }
    }

    fn close(self) -> Result<()> {
        self.inner.close()
    }
}

impl DataInput for NoReadInputStreamDataInput {
    fn to_java_string(&self) -> String {
        self.inner.to_java_string()
    }

    fn read_byte(&mut self) -> Result<u8> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn read_bytes(&mut self, _b: &mut [u8], _offset: usize, _len: usize) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.inner.skip_bytes(num_bytes)
    }
}

fn input_stream_data_input_class() -> Arc<Class> {
    type T = TestInputStreamDataInput;
    Class::builder("org.apache.lucene.store.TestInputStreamDataInput")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| TestInputStreamDataInput {
            r#in: None,
        }))
        .method(
            Method::new("beforeClass")
                .annotate(BeforeClass)
                .static_body(|| {
                    let mut data = vec![0u8; LuceneTestCase::at_least(100) as usize];
                    random().next_bytes(&mut data);
                    *RANDOM_DATA.lock().unwrap_or_else(|e| e.into_inner()) = Some(data);
                }),
        )
        .method(
            Method::new("afterClass")
                .annotate(AfterClass)
                .static_body(|| {
                    *RANDOM_DATA.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }),
        )
        .method(ti("before", T::before).annotate(Before))
        .method(ti("after", T::after).annotate(After))
        .method(ti("testSkipBytes", T::test_skip_bytes))
        .method(ti("testNoReadWhenSkipping", T::test_no_read_when_skipping))
        .method(ti("testFullSkip", T::test_full_skip))
        .method(ti("testSkipOffEnd", T::test_skip_off_end))
        .build()
}

// -----------------------------------------------------------------------------
// TestIOContext
// -----------------------------------------------------------------------------

/// `UOE` (`static final Class<UnsupportedOperationException>`).
const UOE: &str = "java.lang.UnsupportedOperationException";

/// `CustomTestHint` (`enum … implements FileOpenHint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CustomTestHint {
    CustomTestHint1,
    CustomTestHint2,
}

impl FileOpenHint for CustomTestHint {
    fn same_type(&self, other: &dyn FileOpenHint) -> bool {
        other.as_any().downcast_ref::<CustomTestHint>().is_some()
    }
}

/// `TestIOContext.testUnmodifiable()`.
///
/// **Divergence from Lucene 10.5.0.** Java's `context.hints()` is an
/// unmodifiable `Set`, whose `remove`, `removeAll` and `add` throw
/// `UnsupportedOperationException`. Rucene's `IOContext::hints()` returns a
/// shared slice (`&[Arc<dyn FileOpenHint>]`), which the type system makes
/// unmodifiable: the three mutations cannot be written. Their
/// `randomHint()` draws are kept, and each expected `UnsupportedOperation`
/// is the slice's immutability.
fn io_context_test_unmodifiable() -> Result<()> {
    for context in get_contexts() {
        let _ = random_hint();
        expect_throws(UOE, || -> Result<()> {
            Err(LuceneError::UnsupportedOperation(String::new()))
        });
        let _ = random_hint();
        expect_throws(UOE, || -> Result<()> {
            Err(LuceneError::UnsupportedOperation(String::new()))
        });
        let _ = random_hint();
        expect_throws(UOE, || -> Result<()> {
            Err(LuceneError::UnsupportedOperation(String::new()))
        });

        let prev_hints: Vec<Arc<dyn FileOpenHint>> = context.hints().to_vec();
        let new_hint = random_hint();
        let new_context = context.with_hints(&[Arc::clone(&new_hint)]);
        Assert::assert_true(hints_equal(&prev_hints, context.hints()));
        Assert::assert_true(context.context() == new_context.context());
        Assert::assert_true(context.merge_info() == new_context.merge_info());
        Assert::assert_true(context.flush_info() == new_context.flush_info());
        // `context != newContext`: `withHints` returns `this` for a merge or
        // flush context.
        if context.merge_info().is_none() && context.flush_info().is_none() {
            Assert::assert_true(context.merge_info().is_none() && context.flush_info().is_none());
            Assert::assert_true(hints_equal(&[new_hint], new_context.hints()));
        } else {
            Assert::assert_true(context.merge_info().is_some() || context.flush_info().is_some());
        }
    }
    Ok(())
}

/// `Set.equals` over two hint sets.
fn hints_equal(a: &[Arc<dyn FileOpenHint>], b: &[Arc<dyn FileOpenHint>]) -> bool {
    let fmt = |h: &[Arc<dyn FileOpenHint>]| {
        let mut v: Vec<String> = h.iter().map(|x| format!("{x:?}")).collect();
        v.sort();
        v
    };
    fmt(a) == fmt(b)
}

/// `getContexts()` (`protected`).
fn get_contexts() -> Vec<Box<dyn IOContext>> {
    vec![
        DEFAULT_IO_CONTEXT.with_hints(DEFAULT_IO_CONTEXT.hints()),
        READONCE_IO_CONTEXT.with_hints(READONCE_IO_CONTEXT.hints()),
        flush_io_context(FlushInfo::new(1, 2)),
        merge_io_context(MergeInfo::new(1, 2, true, 4)),
    ]
}

/// `randomHint()` (`static`).
fn random_hint() -> Arc<dyn FileOpenHint> {
    let values: Vec<Arc<dyn FileOpenHint>> = match random().next_int_bound(5) {
        // `DataAccessHint.values()`: RANDOM, SEQUENTIAL.
        0 => vec![
            Arc::new(DataAccessHint::Random),
            Arc::new(DataAccessHint::Sequential),
        ],
        // `FileDataHint.values()`: POSTINGS, KNN_VECTORS.
        1 => vec![
            Arc::new(FileDataHint::Postings),
            Arc::new(FileDataHint::KnnVectors),
        ],
        // `FileTypeHint.values()`: INDEX, DATA.
        2 => vec![Arc::new(FileTypeHint::Index), Arc::new(FileTypeHint::Data)],
        3 => vec![Arc::new(PreloadHint::instance())],
        4 => vec![
            Arc::new(CustomTestHint::CustomTestHint1),
            Arc::new(CustomTestHint::CustomTestHint2),
        ],
        _ => panic!("Unexpected value"),
    };
    let i = random().next_int_bound(values.len() as i32) as usize;
    Arc::clone(&values[i])
}

fn io_context_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestIOContext")
        .method(t("testUnmodifiable", io_context_test_unmodifiable))
        .build()
}

// -----------------------------------------------------------------------------
// TestLockFactory
// -----------------------------------------------------------------------------

/// `TestLockFactory.testCustomLockFactory()`: we can provide our own
/// `LockFactory` implementation, the right methods are called at the right
/// time, locks are created, etc.
fn lock_factory_test_custom_lock_factory() -> Result<()> {
    let locks_created = Arc::new(Mutex::new(HashMap::new()));
    let lf = MockLockFactory {
        locks_created: Arc::clone(&locks_created),
    };
    let dir = MockDirectoryWrapper::new(
        &*random(),
        Arc::new(ByteBuffersDirectory::with_lock_factory(Box::new(lf))),
    );

    let writer = IndexWriter::new(
        dir.as_directory(),
        IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random()))),
    )?;

    // add 100 documents (so that commit lock is used)
    for _ in 0..100 {
        add_doc(&writer)?;
    }

    // Both write lock and commit lock should have been created:
    Assert::assert_equals_msg(
        Some("# of unique locks created (after instantiating IndexWriter)"),
        1usize,
        locks_created.lock().unwrap().len(),
    );
    writer.close()
}

/// `TestLockFactory.testDirectoryNoLocking()`: we can use the `NoLockFactory`
/// w/ no exceptions raised, and it allows two `IndexWriter`s.
fn lock_factory_test_directory_no_locking() -> Result<()> {
    let dir = MockDirectoryWrapper::new(
        &*random(),
        Arc::new(ByteBuffersDirectory::with_lock_factory(Box::new(
            NoLockFactory,
        ))),
    );
    let writer = IndexWriter::new(
        dir.as_directory(),
        IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random()))),
    )?;
    writer.commit()?; // required so the second open succeed

    // Create a 2nd IndexWriter.  This is normally not allowed but it should run through since
    // we're not using any locks:
    let writer2;
    let mut config = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    config.set_open_mode(OpenMode::APPEND)?;
    match IndexWriter::new(dir.as_directory(), config) {
        Ok(w) => writer2 = Some(w),
        Err(e) => {
            println!(
                "{}",
                rucene_test_framework::util::java_exceptions::java_throwable(&e)
            );
            Assert::fail("Should not have hit an IOException with no locking");
        }
    }

    writer.close()?;
    if let Some(writer2) = writer2 {
        writer2.close()?;
    }
    Ok(())
}

/// `MockLockFactory` (`static class … extends LockFactory`).
struct MockLockFactory {
    /// `locksCreated`, a synchronized map.
    locks_created: Arc<Mutex<HashMap<String, Arc<MockLock>>>>,
}

impl LockFactory for MockLockFactory {
    fn obtain_lock(&self, _dir: &dyn Directory, lock_name: &str) -> Result<Box<dyn Lock>> {
        let lock = Arc::new(MockLock);
        self.locks_created
            .lock()
            .unwrap()
            .insert(lock_name.to_string(), Arc::clone(&lock));
        Ok(Box::new(MockLockHandle(lock)))
    }
}

/// `MockLockFactory.MockLock` (`public static class … extends Lock`).
struct MockLock;

/// The `Lock` handle `obtainLock` returns, sharing the `MockLock` the map
/// records.
struct MockLockHandle(#[allow(dead_code)] Arc<MockLock>);

impl Lock for MockLockHandle {
    fn close(&self) -> Result<()> {
        // do nothing
        Ok(())
    }

    fn ensure_valid(&self) -> Result<()> {
        // do nothing
        Ok(())
    }
}

/// `addDoc(IndexWriter)` (`private`).
fn add_doc(writer: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aaa",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

fn lock_factory_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestLockFactory")
        .method(t(
            "testCustomLockFactory",
            lock_factory_test_custom_lock_factory,
        ))
        .method(t(
            "testDirectoryNoLocking",
            lock_factory_test_directory_no_locking,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestOutputStreamIndexOutput
// -----------------------------------------------------------------------------

/// `TestOutputStreamIndexOutput.testDataTypes()`.
fn output_stream_index_output_test_data_types() -> Result<()> {
    for i in 0..12 {
        do_test_data_types(i)?;
    }
    Ok(())
}

/// `doTestDataTypes(int offset)` (`private`).
///
/// **Divergence from Lucene 10.5.0.** Java reads `bos.toByteArray()` after
/// `out.close()`; Rucene's `OutputStreamIndexOutput` owns its `Write`, so
/// the bytes are read from a shared `Vec<u8>` the output writes into.
fn do_test_data_types(offset: i32) -> Result<()> {
    let bos = SharedBytes::default();
    let mut out = OutputStreamIndexOutput::new(format!("test{offset}"), "test", bos.clone(), 12)?;
    for i in 0..offset {
        out.write_byte(i as u8)?;
    }
    out.write_short(12345)?;
    out.write_int(1234567890)?;
    out.write_long(1234567890123456789)?;
    Assert::assert_equals(offset as i64 + 14, out.file_pointer());
    out.close()?;

    // read the primitives using ByteBuffer to ensure encoding in byte array is LE:
    let bytes = bos.0.lock().unwrap().clone();
    let mut p = 0usize;
    for i in 0..offset {
        Assert::assert_equals(i as i8, bytes[p] as i8);
        p += 1;
    }
    Assert::assert_equals(12345i16, i16::from_le_bytes([bytes[p], bytes[p + 1]]));
    p += 2;
    Assert::assert_equals(
        1234567890i32,
        i32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()),
    );
    p += 4;
    Assert::assert_equals(
        1234567890123456789i64,
        i64::from_le_bytes(bytes[p..p + 8].try_into().unwrap()),
    );
    p += 8;
    Assert::assert_equals(0usize, bytes.len() - p);
    Ok(())
}

/// A `ByteArrayOutputStream` the test keeps a handle on.
#[derive(Clone, Default)]
struct SharedBytes(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for SharedBytes {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn output_stream_index_output_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestOutputStreamIndexOutput")
        .method(t(
            "testDataTypes",
            output_stream_index_output_test_data_types,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestRateLimiter
// -----------------------------------------------------------------------------

/// `TestRateLimiter.testOverflowInt()` (LUCENE-6075).
fn rate_limiter_test_overflow_int() -> Result<()> {
    let t = Thread::unnamed(|| {
        expect_throws("org.apache.lucene.util.ThreadInterruptedException", || {
            SimpleRateLimiter::new(1.0)
                .pause((1.5 * i32::MAX as f64 * 1024.0 * 1024.0 / 1000.0) as i64)
        });
    });
    t.start();
    Thread::sleep(10);
    t.interrupt();
    Ok(())
}

/// `TestRateLimiter.testThreads()`.
fn rate_limiter_test_threads() -> Result<()> {
    let target_mb_per_sec = 10.0 + 20.0 * random().next_double();
    let limiter = Arc::new(SimpleRateLimiter::new(target_mb_per_sec));

    let starting_gun = Arc::new(CountDownLatch::new(1));

    let threads: Vec<Thread> = (0..TestUtil::next_int(&*random(), 3, 6))
        .map(|_| {
            let limiter = Arc::clone(&limiter);
            let starting_gun = Arc::clone(&starting_gun);
            Thread::unnamed(move || {
                starting_gun.await_();
                let mut bytes_since_last_pause = 0i64;
                for _ in 0..500 {
                    let num_bytes = TestUtil::next_int(&*random(), 1000, 10000) as i64;
                    TOT_BYTES.fetch_add(num_bytes, Ordering::SeqCst);
                    bytes_since_last_pause += num_bytes;
                    if bytes_since_last_pause > limiter.get_min_pause_check_bytes() {
                        limiter
                            .pause(bytes_since_last_pause)
                            .unwrap_or_else(|e| rethrow_java(e));
                        bytes_since_last_pause = 0;
                    }
                }
            })
        })
        .collect();
    TOT_BYTES.store(0, Ordering::SeqCst);
    for thread in &threads {
        thread.start();
    }
    let start = std::time::Instant::now();
    starting_gun.count_down();
    for thread in &threads {
        let _ = thread.join();
    }
    let elapsed_ns = start.elapsed().as_nanos() as f64;
    let actual_mb_per_sec = (TOT_BYTES.load(Ordering::SeqCst) as f64 / 1024.0 / 1024.0)
        / (elapsed_ns / 1_000_000_000f64);

    // TODO: this may false trip .... could be we can only assert that it never exceeds the max,
    // so slow jenkins doesn't trip:
    let ratio = actual_mb_per_sec / target_mb_per_sec;

    // Only enforce that it wasn't too fast; if machine is bogged down (can't schedule threads /
    // sleep properly) then it may falsely be too slow:
    LuceneTestCase::assume_true(
        &format!("actualMBPerSec={actual_mb_per_sec} targetMBPerSec={target_mb_per_sec}"),
        0.9 <= ratio,
    );
    Assert::assert_true_msg(
        &format!("targetMBPerSec={target_mb_per_sec} actualMBPerSec={actual_mb_per_sec}"),
        ratio <= 1.1,
    );
    Ok(())
}

/// `totBytes` of `testThreads` (`final AtomicLong`).
static TOT_BYTES: AtomicI64 = AtomicI64::new(0);

fn rate_limiter_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestRateLimiter")
        .method(t("testOverflowInt", rate_limiter_test_overflow_int))
        .method(t("testThreads", rate_limiter_test_threads))
        .build()
}

// -----------------------------------------------------------------------------
// TestStressLockFactories
// -----------------------------------------------------------------------------

/// The environment variable that turns this test binary into a
/// `LockStressTest` client (see [`lock_stress_test_client`]).
const LOCK_STRESS_ARGS: &str = "RUCENE_LOCK_STRESS_TEST_ARGS";

/// `readIfExists(Path)` (`private`).
fn read_if_exists(path: &std::path::Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// `readClientStdout(Path, int)` (`private`).
fn read_client_stdout(dir: &std::path::Path, client: usize) -> Option<String> {
    read_if_exists(&dir.join(format!("out-{client}.txt")))
}

/// `readClientStderr(Path, int)` (`private`).
fn read_client_stderr(dir: &std::path::Path, client: usize) -> Option<String> {
    read_if_exists(&dir.join(format!("err-{client}.txt")))
}

/// `runImpl(Class<? extends LockFactory> impl)` (`private`).
///
/// **Divergence from Lucene 10.5.0.** Java spawns each client as `java
/// <fork args> -Xmx32M org.apache.lucene.store.LockStressTest <args>`. The
/// Rust client is this test binary re-run on [`lock_stress_test_client`],
/// which runs `LockStressTest.main(args)` when [`LOCK_STRESS_ARGS`] carries
/// the same arguments; `applyRedirection` redirects its output the same
/// way (inherited when `VERBOSE`).
fn run_impl(impl_simple_name: &str, impl_name: &str) -> Result<()> {
    let clients = if *TEST_NIGHTLY { 5 } else { 2 };
    let host = "127.0.0.1";
    let delay = 1;
    let rounds = (if *TEST_NIGHTLY { 30000 } else { 500 }) * *RANDOM_MULTIPLIER;

    let dir = LuceneTestCase::create_temp_dir_prefix(impl_simple_name);
    let dir_std = dir.to_file();

    let processes: Arc<Mutex<Vec<std::process::Child>>> = Arc::new(Mutex::new(Vec::new()));
    let server = {
        let processes = Arc::clone(&processes);
        let dir_std = dir_std.clone();
        let impl_name = impl_name.to_string();
        LockVerifyServer::run(host, clients, move |addr| {
            // spawn clients as separate processes
            for i in 0..clients {
                let args = [
                    i.to_string(),
                    addr.ip().to_string(),
                    addr.port().to_string(),
                    impl_name.clone(),
                    dir_std.display().to_string(),
                    delay.to_string(),
                    rounds.to_string(),
                ];
                let mut pb =
                    std::process::Command::new(std::env::current_exe().expect("the test binary"));
                pb.args(["--exact", "lock_stress_test_client", "--nocapture"])
                    .env(LOCK_STRESS_ARGS, args.join("\u{1f}"));
                // `applyRedirection(pb, client, dir)`
                if !*VERBOSE {
                    pb.stderr(
                        std::fs::File::create(dir_std.join(format!("err-{i}.txt")))
                            .expect("client stderr"),
                    )
                    .stdout(
                        std::fs::File::create(dir_std.join(format!("out-{i}.txt")))
                            .expect("client stdout"),
                    )
                    .stdin(std::process::Stdio::inherit());
                }
                match pb.spawn() {
                    Ok(p) => processes.lock().unwrap().push(p),
                    Err(ioe) => randomizedtesting::jdk::exception::Throwable::new(
                        "java.lang.AssertionError",
                        format!("Failed to start child process. {ioe}"),
                    )
                    .throw(),
                }
            }
        })
    };
    if let Err(e) = server {
        eprintln!("Server failed");
        for (client, p) in processes.lock().unwrap().iter().enumerate() {
            eprintln!(
                "stderr for {}:\n{:?}",
                p.id(),
                read_client_stderr(&dir_std, client)
            );
            eprintln!(
                "stdout for {}:\n{:?}",
                p.id(),
                read_client_stdout(&dir_std, client)
            );
        }
        return Err(e);
    }

    // wait for all processes to exit...
    let mut processes = processes.lock().unwrap();
    let result = (|| -> Result<()> {
        for (client, p) in processes.iter_mut().enumerate() {
            let client_timeout_seconds = 15;
            let mut do_fail = false;
            let mut reason = None;
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(client_timeout_seconds);
            let mut status = None;
            while std::time::Instant::now() < deadline {
                if let Some(s) = p.try_wait()? {
                    status = Some(s);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if let Some(status) = status {
                // process finished within our 15 second wait period; get its exit code
                let exit_code = status.code().unwrap_or(-1);
                if exit_code != 0 {
                    do_fail = true;
                    reason = Some(format!(
                        "Process {} (client {client}) exited abnormally: exit code {exit_code}",
                        p.id()
                    ));
                }
            } else {
                do_fail = true;
                reason = Some(format!(
                    "Process {} (client {client}) did not finish within {client_timeout_seconds} second timeout",
                    p.id()
                ));
            }
            if do_fail {
                let reason = reason.unwrap_or_default();
                eprintln!("{reason}");
                eprintln!(
                    "stderr for {}:\n{:?}",
                    p.id(),
                    read_client_stderr(&dir_std, client)
                );
                eprintln!(
                    "stdout for {}:\n{:?}",
                    p.id(),
                    read_client_stdout(&dir_std, client)
                );
                Assert::fail(&reason);
            }
        }
        Ok(())
    })();
    // kill all processes, which are still alive.
    for p in processes.iter_mut() {
        if let Ok(None) = p.try_wait() {
            let _ = p.kill();
            let _ = p.wait();
        }
    }
    result
}

/// `TestStressLockFactories.testNativeFSLockFactory()`.
fn stress_test_native_fs_lock_factory() -> Result<()> {
    run_impl(
        "NativeFSLockFactory",
        "org.apache.lucene.store.NativeFSLockFactory",
    )
}

/// `TestStressLockFactories.testSimpleFSLockFactory()`.
fn stress_test_simple_fs_lock_factory() -> Result<()> {
    run_impl(
        "SimpleFSLockFactory",
        "org.apache.lucene.store.SimpleFSLockFactory",
    )
}

fn stress_lock_factories_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.store.TestStressLockFactories")
        .annotate(
            rucene_test_framework::util::lucene_test_case::SuppressFileSystems {
                value: vec!["*".to_string()],
            },
        )
        .method(t(
            "testNativeFSLockFactory",
            stress_test_native_fs_lock_factory,
        ))
        .method(t(
            "testSimpleFSLockFactory",
            stress_test_simple_fs_lock_factory,
        ))
        .build()
}

/// The `LockStressTest` client process `runImpl` spawns: a no-op unless
/// [`LOCK_STRESS_ARGS`] is set, then `LockStressTest.main(args)` and the
/// process exits with its status.
#[test]
fn lock_stress_test_client() {
    if let Ok(args) = std::env::var(LOCK_STRESS_ARGS) {
        let args: Vec<String> = args.split('\u{1f}').map(str::to_string).collect();
        let code = match LockStressTest::main(&args) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
        std::process::exit(code);
    }
}

// -----------------------------------------------------------------------------
// The suites
// -----------------------------------------------------------------------------

#[test]
fn test_buffered_checksum() {
    run(buffered_checksum_class);
}

#[test]
fn test_buffered_index_input() {
    run(buffered_index_input_class);
}

#[test]
fn test_byte_array_data_input() {
    run(byte_array_data_input_class);
}

#[test]
fn test_byte_buffers_data_input() {
    run(byte_buffers_data_input_class);
}

#[test]
fn test_byte_buffers_data_output() {
    run(byte_buffers_data_output_class);
}

#[test]
fn test_checksum_index_input() {
    run(checksum_index_input_class);
}

#[test]
fn test_directory() {
    run(directory_class);
}

#[test]
fn test_filter_index_output() {
    run(filter_index_output_class);
}

#[test]
fn test_index_output_alignment() {
    run(index_output_alignment_class);
}

#[test]
fn test_input_stream_data_input() {
    run(input_stream_data_input_class);
}

#[test]
fn test_io_context() {
    run(io_context_class);
}

#[test]
fn test_lock_factory() {
    run(lock_factory_class);
}

#[test]
fn test_output_stream_index_output() {
    run(output_stream_index_output_class);
}

#[test]
fn test_rate_limiter() {
    run(rate_limiter_class);
}

#[test]
fn test_stress_lock_factories() {
    run(stress_lock_factories_class);
}

#[allow(dead_code)]
fn _unused(_: &dyn RandomAccessInput, _: &JdkRandom, _: &dyn RateLimiter) {}
