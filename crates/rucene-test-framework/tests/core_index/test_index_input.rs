//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexInput.java`.
//!
//! * The overridable `getIndexInput(len)` is the instance's
//!   `get_index_input` function, which `TestFilterIndexInput` replaces.
//! * The static `INTS`, `LONGS` and `RANDOM_TEST_BYTES` live in a
//!   process-wide cell that `beforeClass` fills and `afterClass` clears.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::error::{LuceneError, Result};
use rucene::store::{ByteArrayDataInput, ByteArrayDataOutput, DataInput, DataOutput, IndexInput};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, random};
use crate::support::{npe, static_hook};

/// `READ_TEST_BYTES`.
pub const READ_TEST_BYTES: &[u8] = &[
    0x80, 0x01, 0xFF, 0x7F, 0x80, 0x80, 0x01, 0x81, 0x80, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0x07, 0xFF,
    0xFF, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0x07, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0x7F, 0x06, b'L', b'u', b'c', b'e', b'n', b'e',
    // 2-byte UTF-8 (U+00BF "INVERTED QUESTION MARK")
    0x02, 0xC2, 0xBF, 0x0A, b'L', b'u', 0xC2, 0xBF, b'c', b'e', 0xC2, 0xBF, b'n', b'e',
    // 3-byte UTF-8 (U+2620 "SKULL AND CROSSBONES")
    0x03, 0xE2, 0x98, 0xA0, 0x0C, b'L', b'u', 0xE2, 0x98, 0xA0, b'c', b'e', 0xE2, 0x98, 0xA0, b'n',
    b'e',
    // surrogate pairs
    // (U+1D11E "MUSICAL SYMBOL G CLEF")
    // (U+1D160 "MUSICAL SYMBOL EIGHTH NOTE")
    0x04, 0xF0, 0x9D, 0x84, 0x9E, 0x08, 0xF0, 0x9D, 0x84, 0x9E, 0xF0, 0x9D, 0x85, 0xA0, 0x0E, b'L',
    b'u', 0xF0, 0x9D, 0x84, 0x9E, b'c', b'e', 0xF0, 0x9D, 0x85, 0xA0, b'n', b'e',
    // null bytes
    0x01, 0x00, 0x08, b'L', b'u', 0x00, b'c', b'e', 0x00, b'n', b'e',
];

/// `COUNT = RANDOM_MULTIPLIER * 65536`.
fn count() -> usize {
    (*RANDOM_MULTIPLIER * 65536) as usize
}

/// The static `INTS`, `LONGS` and `RANDOM_TEST_BYTES`.
#[derive(Default)]
pub struct Statics {
    pub ints: Option<Vec<i32>>,
    pub longs: Option<Vec<i64>>,
    pub random_test_bytes: Option<Vec<u8>>,
}

static STATICS: Mutex<Statics> = Mutex::new(Statics {
    ints: None,
    longs: None,
    random_test_bytes: None,
});

/// `RANDOM_TEST_BYTES`.
pub fn random_test_bytes() -> Vec<u8> {
    STATICS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .random_test_bytes
        .clone()
        .unwrap_or_else(|| npe("RANDOM_TEST_BYTES"))
}

/// The instance of `TestIndexInput` (or of a subclass): its
/// `getIndexInput(len)`.
pub struct IndexInputInstance {
    pub get_index_input: fn(i64) -> Box<dyn IndexInput>,
}

impl IndexInputInstance {
    /// `new TestIndexInput()`.
    pub fn new() -> Self {
        IndexInputInstance {
            get_index_input: get_index_input,
        }
    }
}

/// A test method over the instance.
pub fn instance_test(name: &str, body: fn(&IndexInputInstance) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.io.IOException"])
        .body(move |inst: &mut IndexInputInstance| body(inst).unwrap_or_else(|e| rethrow_java(e)))
}

/// The builder of `TestIndexInput`.
fn builder() -> ClassBuilder {
    Class::builder("org.apache.lucene.index.TestIndexInput")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(IndexInputInstance::new))
        .method(static_hook("beforeClass", BeforeClass, before_class))
        .method(static_hook("afterClass", AfterClass, after_class))
        .method(instance_test("testRawIndexInputRead", |_| {
            test_raw_index_input_read()
        }))
        .method(instance_test("testByteArrayDataInput", |_| {
            test_byte_array_data_input()
        }))
        .method(instance_test(
            "testNoReadOnSkipBytes",
            test_no_read_on_skip_bytes,
        ))
}

/// `TestIndexInput`, which `TestFilterIndexInput` extends.
pub fn class_def() -> Arc<Class> {
    builder().build()
}

/// `@BeforeClass public static void beforeClass()`.
fn before_class() -> Result<()> {
    let random = random();
    let count = count();
    let mut ints = vec![0i32; count];
    let mut longs = vec![0i64; count];
    let mut random_test_bytes = vec![0u8; count * (5 + 4 + 9 + 8)];
    {
        let mut bdo = ByteArrayDataOutput::new(&mut random_test_bytes);
        for i in 0..count {
            ints[i] = random.next_int();
            let i1 = ints[i];
            bdo.write_v_int(i1)?;
            bdo.write_int(i1)?;

            let l1 = if LuceneTestCase::rarely() {
                // a long with lots of zeroes at the end
                longs[i] = TestUtil::next_long(&*random, 0, i32::MAX as i64) << 32;
                longs[i]
            } else {
                longs[i] = TestUtil::next_long(&*random, 0, i64::MAX);
                longs[i]
            };
            bdo.write_v_long(l1)?;
            bdo.write_long(l1)?;
        }
    }
    let mut s = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    s.ints = Some(ints);
    s.longs = Some(longs);
    s.random_test_bytes = Some(random_test_bytes);
    Ok(())
}

/// `@AfterClass public static void afterClass()`.
fn after_class() -> Result<()> {
    let mut s = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    s.ints = None;
    s.longs = None;
    s.random_test_bytes = None;
    Ok(())
}

/// `checkReads(is, expectedEx)`.
pub fn check_reads(is: &mut dyn DataInput, _expected_ex: &str) -> Result<()> {
    assert_equals(&128, &is.read_v_int()?);
    assert_equals(&16383, &is.read_v_int()?);
    assert_equals(&16384, &is.read_v_int()?);
    assert_equals(&16385, &is.read_v_int()?);
    assert_equals(&i32::MAX, &is.read_v_int()?);
    assert_equals(&-1, &is.read_v_int()?);
    assert_equals(&(i32::MAX as i64), &is.read_v_long()?);
    assert_equals(&i64::MAX, &is.read_v_long()?);
    assert_equals("Lucene", is.read_string()?.as_str());

    assert_equals("\u{00BF}", is.read_string()?.as_str());
    assert_equals("Lu\u{00BF}ce\u{00BF}ne", is.read_string()?.as_str());

    assert_equals("\u{2620}", is.read_string()?.as_str());
    assert_equals("Lu\u{2620}ce\u{2620}ne", is.read_string()?.as_str());

    assert_equals("\u{1D11E}", is.read_string()?.as_str());
    assert_equals("\u{1D11E}\u{1D160}", is.read_string()?.as_str());
    assert_equals("Lu\u{1D11E}ce\u{1D160}ne", is.read_string()?.as_str());

    assert_equals("\u{0000}", is.read_string()?.as_str());
    assert_equals("Lu\u{0000}ce\u{0000}ne", is.read_string()?.as_str());
    Ok(())
}

/// `checkRandomReads(is)`.
pub fn check_random_reads(is: &mut dyn DataInput) -> Result<()> {
    let (ints, longs) = {
        let s = STATICS.lock().unwrap_or_else(|e| e.into_inner());
        (
            s.ints.clone().unwrap_or_else(|| npe("INTS")),
            s.longs.clone().unwrap_or_else(|| npe("LONGS")),
        )
    };
    for i in 0..count() {
        assert_equals(&ints[i], &is.read_v_int()?);
        assert_equals(&ints[i], &is.read_int()?);
        assert_equals(&longs[i], &is.read_v_long()?);
        assert_equals(&longs[i], &is.read_long()?);
    }
    Ok(())
}

/// `checkSeeksAndSkips(is, random)`.
pub fn check_seeks_and_skips(is: &mut dyn IndexInput, random: &dyn Random) -> Result<()> {
    let len = is.length();

    let iterations = if *TEST_NIGHTLY { 1_000 } else { 10 };
    for _ in 0..iterations {
        is.seek(0)?; // make sure we're at the start

        let mut curr: i64 = 0;
        while curr < len {
            let max_skip_to = len - 1;
            // if we're close to the end, just skip all the way
            let skip_to = if len - curr < 10 {
                max_skip_to
            } else {
                TestUtil::next_long(random, curr, max_skip_to)
            };
            let skip_delta = skip_to - curr;

            // first reposition using seek
            let start_byte1 = is.read_byte()? as i8;
            is.seek(skip_to)?;
            let end_byte1 = is.read_byte()? as i8;

            // do the same thing but with skipBytes
            is.seek(curr)?;
            let start_byte2 = is.read_byte()? as i8;
            is.seek(curr)?;
            is.skip_bytes(skip_delta)?;
            let end_byte2 = is.read_byte()? as i8;

            assert_equals(&start_byte1, &start_byte2);
            assert_equals(&end_byte1, &end_byte2);
            // +1 since we read the byte we seek/skip to
            assert_equals(&(curr + skip_delta + 1), &is.file_pointer());

            curr = is.file_pointer();
        }
    }
    Ok(())
}

/// this test checks the IndexInput methods of any impl
/// Port of `TestIndexInput.testRawIndexInputRead`.
fn test_raw_index_input_read() -> Result<()> {
    for _ in 0..10 {
        let random = random();
        let mut dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();
        let mut os = d.create_output("foo", &*LuceneTestCase::new_io_context(&*random))?;
        os.write_bytes(READ_TEST_BYTES, 0, READ_TEST_BYTES.len())?;
        os.close()?;
        let mut is = d.open_input("foo", &*LuceneTestCase::new_io_context(&*random))?;
        check_reads(&mut *is, "java.io.IOException")?;
        check_seeks_and_skips(&mut *is, &*random)?;
        is.close()?;

        let bytes = random_test_bytes();
        let mut os = d.create_output("bar", &*LuceneTestCase::new_io_context(&*random))?;
        os.write_bytes(&bytes, 0, bytes.len())?;
        os.close()?;
        let mut is = d.open_input("bar", &*LuceneTestCase::new_io_context(&*random))?;
        check_random_reads(&mut *is)?;
        check_seeks_and_skips(&mut *is, &*random)?;
        is.close()?;
        drop(d);
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestIndexInput.testByteArrayDataInput`.
fn test_byte_array_data_input() -> Result<()> {
    let mut is = ByteArrayDataInput::new(READ_TEST_BYTES);
    check_reads(&mut is, "java.lang.RuntimeException")?;
    let bytes = random_test_bytes();
    let mut is = ByteArrayDataInput::new(&bytes);
    check_random_reads(&mut is)
}

/// Port of `TestIndexInput.testNoReadOnSkipBytes`.
fn test_no_read_on_skip_bytes(inst: &IndexInputInstance) -> Result<()> {
    let len = if *TEST_NIGHTLY { i64::MAX } else { 1_000_000 };
    let max_seek_pos = len - 1;
    let mut is = (inst.get_index_input)(len);

    while is.file_pointer() < max_seek_pos {
        let seek_pos = TestUtil::next_long(&*random(), is.file_pointer(), max_seek_pos);
        let skip_delta = seek_pos - is.file_pointer();
        is.skip_bytes(skip_delta)?;
        assert_equals(&seek_pos, &is.file_pointer());
    }
    Ok(())
}

/// `getIndexInput(len)`.
pub fn get_index_input(len: i64) -> Box<dyn IndexInput> {
    Box::new(InterceptingIndexInput::new("foo", len))
}

/// Mock IndexInput that just tracks a position (which responds to seek/skip) and throws if
/// #readByte or #readBytes are called, ensuring seek/skip doesn't invoke reads.
pub struct InterceptingIndexInput {
    resource_description: String,
    pos: i64,
    len: i64,
}

impl InterceptingIndexInput {
    /// `new InterceptingIndexInput(resourceDescription, len)`.
    pub fn new(resource_description: &str, len: i64) -> Self {
        InterceptingIndexInput {
            resource_description: resource_description.to_string(),
            pos: 0,
            len,
        }
    }
}

impl DataInput for InterceptingIndexInput {
    /// The inherited `IndexInput.toString()`: the resource description.
    fn to_java_string(&self) -> String {
        self.resource_description.clone()
    }

    fn read_byte(&mut self) -> Result<u8> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn read_bytes(&mut self, _b: Option<&mut [u8]>, _offset: usize, _len: usize) -> Result<()> {
        let _b = rucene::store::require_array(_b)?;
        Err(LuceneError::unsupported_operation(String::new()))
    }

    /// The inherited `IndexInput.skipBytes(numBytes)`: `seek(getFilePointer()
    /// + numBytes)` after the argument check.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        self.pos += num_bytes;
        Ok(())
    }
}

impl IndexInput for InterceptingIndexInput {
    fn close(&mut self) -> Result<()> {
        // no-op
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.pos
    }

    fn length(&self) -> i64 {
        self.len
    }

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
        Err(LuceneError::unsupported_operation(String::new()))
    }

    /// `IndexInput.clone()` is `super.clone()`: a copy of the fields.
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(InterceptingIndexInput {
            resource_description: self.resource_description.clone(),
            pos: self.pos,
            len: self.len,
        }))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}
