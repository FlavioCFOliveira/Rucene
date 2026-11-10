//! Port of `lucene/core/src/test/org/apache/lucene/codecs/TestCodecUtil.java`.
//!
//! tests for codecutil methods
//!
//! Java writes through a `ByteBuffersIndexOutput` over a
//! `ByteBuffersDataOutput out` it keeps, and reads `out.toDataInput()` after
//! closing the output. Rucene's output owns its data output, so the bytes it
//! holds are copied out just before it closes ([`close`]) and read back
//! through a `ByteBuffersDataOutput` of the same content ([`temp_input`]).

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::codec_util::{
    check_footer_with_prior, check_header, check_index_header, checksum_entire_file, footer_length,
    header_length, index_header_length, read_crc, retrieve_checksum,
    retrieve_checksum_expected_length, write_be_int, write_be_long, write_crc, write_footer,
    write_header, write_index_header, FOOTER_MAGIC,
};
use rucene::error::{LuceneError, Result};
use rucene::store::{
    BufferedChecksumIndexInput, ByteBuffersDataOutput, ByteBuffersIndexInput,
    ByteBuffersIndexOutput, DataInput, DataOutput, IndexInput, IndexOutput, DEFAULT_IO_CONTEXT,
};
use rucene::util::string_helper::StringHelper;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, test};

/// `TestCodecUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.TestCodecUtil",
        vec![
            test("testHeaderLength", test_header_length),
            test("testWriteTooLongHeader", test_write_too_long_header),
            test("testWriteNonAsciiHeader", test_write_non_ascii_header),
            test("testReadHeaderWrongMagic", test_read_header_wrong_magic),
            test("testChecksumEntireFile", test_checksum_entire_file),
            test("testCheckFooterValid", test_check_footer_valid),
            test(
                "testCheckFooterValidAtFooter",
                test_check_footer_valid_at_footer,
            ),
            test(
                "testCheckFooterValidPastFooter",
                test_check_footer_valid_past_footer,
            ),
            test("testCheckFooterInvalid", test_check_footer_invalid),
            test("testSegmentHeaderLength", test_segment_header_length),
            test("testWriteTooLongSuffix", test_write_too_long_suffix),
            test("testWriteVeryLongSuffix", test_write_very_long_suffix),
            test("testWriteNonAsciiSuffix", test_write_non_ascii_suffix),
            test("testReadBogusCRC", test_read_bogus_crc),
            test("testWriteBogusCRC", test_write_bogus_crc),
            test(
                "testTruncatedFileThrowsCorruptIndexException",
                test_truncated_file_throws_corrupt_index_exception,
            ),
            test("testRetrieveChecksum", test_retrieve_checksum),
        ],
    )
}

/// The content of Java's `ByteBuffersDataOutput out` once the output closed.
type SharedOut = Vec<u8>;

/// `new ByteBuffersIndexOutput(out, "temp", "temp")` over a fresh
/// `ByteBuffersDataOutput out`.
fn temp_output() -> ByteBuffersIndexOutput {
    ByteBuffersIndexOutput::new(ByteBuffersDataOutput::new(), "temp", "temp")
}

/// `output.close()`, answering the content of `out`.
fn close(output: &mut ByteBuffersIndexOutput) -> Result<SharedOut> {
    let bytes = output.to_array_copy()?;
    output.close()?;
    Ok(bytes)
}

/// `new ByteBuffersIndexInput(out.toDataInput(), "temp")`.
fn temp_input(out: &SharedOut) -> Result<ByteBuffersIndexInput> {
    let mut data = ByteBuffersDataOutput::new();
    data.write_bytes(out, 0, out.len())?;
    Ok(ByteBuffersIndexInput::new(data.to_data_input()?, "temp"))
}

/// `new RuntimeException("fake exception")`.
fn fake_exception() -> LuceneError {
    LuceneError::unchecked("java.lang.RuntimeException", "fake exception")
}

/// `CodecUtil.checkFooter(input, mine)`, which always throws.
fn check_footer_throwing(input: &mut BufferedChecksumIndexInput, mine: LuceneError) -> Result<()> {
    Err(check_footer_with_prior(input, mine))
}

/// `e.getMessage()` of an expected throwable.
fn message(e: &randomizedtesting::jdk::exception::Throwable) -> String {
    e.message().unwrap_or_default().to_string()
}

/// Port of `TestCodecUtil.testHeaderLength`.
fn test_header_length() -> Result<()> {
    let mut output = temp_output();
    write_header(&mut output, "FooBar", 5)?;
    output.write_string("this is the data")?;
    let out = close(&mut output)?;

    let mut input = temp_input(&out)?;
    input.seek(header_length("FooBar") as i64)?;
    assert_equals("this is the data", input.read_string()?.as_str());
    input.close()
}

/// Port of `TestCodecUtil.testWriteTooLongHeader`.
fn test_write_too_long_header() -> Result<()> {
    let mut too_long = String::new();
    for _ in 0..128 {
        too_long.push('a');
    }
    let mut output = temp_output();
    expect_throws("java.lang.IllegalArgumentException", || {
        write_header(&mut output, &too_long, 5)
    });
    Ok(())
}

/// Port of `TestCodecUtil.testWriteNonAsciiHeader`.
fn test_write_non_ascii_header() -> Result<()> {
    let mut output = temp_output();
    expect_throws("java.lang.IllegalArgumentException", || {
        write_header(&mut output, "\u{1234}", 5)
    });
    Ok(())
}

/// Port of `TestCodecUtil.testReadHeaderWrongMagic`.
fn test_read_header_wrong_magic() -> Result<()> {
    let mut output = temp_output();
    output.write_int(1234)?;
    let out = close(&mut output)?;

    let mut input = temp_input(&out)?;
    expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        check_header(&mut input, "bogus", 1, 1)
    });
    Ok(())
}

/// Port of `TestCodecUtil.testChecksumEntireFile`.
fn test_checksum_entire_file() -> Result<()> {
    let mut output = temp_output();
    write_header(&mut output, "FooBar", 5)?;
    output.write_string("this is the data")?;
    write_footer(&mut output)?;
    let out = close(&mut output)?;

    let mut input = temp_input(&out)?;
    checksum_entire_file(&mut input)?;
    input.close()
}

/// The data of `testCheckFooterValid*`: a header, a string and a footer.
fn header_data_footer() -> Result<SharedOut> {
    let mut output = temp_output();
    write_header(&mut output, "FooBar", 5)?;
    output.write_string("this is the data")?;
    write_footer(&mut output)?;
    let out = close(&mut output)?;
    Ok(out)
}

/// Port of `TestCodecUtil.testCheckFooterValid`.
fn test_check_footer_valid() -> Result<()> {
    let out = header_data_footer()?;

    let mut input = BufferedChecksumIndexInput::new(Box::new(temp_input(&out)?));
    let mine = fake_exception();
    let expected = expect_throws("java.lang.RuntimeException", || {
        check_footer_throwing(&mut input, mine)
    });
    assert_equals("fake exception", message(&expected).as_str());
    let suppressed = expected.suppressed();
    assert_equals(&1, &suppressed.len());
    Assert::assert_true(message(&suppressed[0]).contains("checksum passed"));
    input.close()
}

/// Port of `TestCodecUtil.testCheckFooterValidAtFooter`.
fn test_check_footer_valid_at_footer() -> Result<()> {
    let out = header_data_footer()?;

    let mut input = BufferedChecksumIndexInput::new(Box::new(temp_input(&out)?));
    check_header(&mut input, "FooBar", 5, 5)?;
    assert_equals("this is the data", input.read_string()?.as_str());
    let mine = fake_exception();
    let expected = expect_throws("java.lang.RuntimeException", || {
        check_footer_throwing(&mut input, mine)
    });
    assert_equals("fake exception", message(&expected).as_str());
    let suppressed = expected.suppressed();
    assert_equals(&1, &suppressed.len());
    Assert::assert_true(message(&suppressed[0]).contains("checksum passed"));
    input.close()
}

/// Port of `TestCodecUtil.testCheckFooterValidPastFooter`.
fn test_check_footer_valid_past_footer() -> Result<()> {
    let out = header_data_footer()?;

    let mut input = BufferedChecksumIndexInput::new(Box::new(temp_input(&out)?));
    check_header(&mut input, "FooBar", 5, 5)?;
    assert_equals("this is the data", input.read_string()?.as_str());
    // bogusly read a byte too far (can happen)
    input.read_byte()?;
    let mine = fake_exception();
    let expected = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        check_footer_throwing(&mut input, mine)
    });
    Assert::assert_true(message(&expected).contains("checksum status indeterminate"));
    let suppressed = expected.suppressed();
    assert_equals(&1, &suppressed.len());
    assert_equals("fake exception", message(&suppressed[0]).as_str());
    input.close()
}

/// Port of `TestCodecUtil.testCheckFooterInvalid`.
fn test_check_footer_invalid() -> Result<()> {
    let mut output = temp_output();
    write_header(&mut output, "FooBar", 5)?;
    output.write_string("this is the data")?;
    write_be_int(&mut output, FOOTER_MAGIC)?;
    write_be_int(&mut output, 0)?;
    write_be_long(&mut output, 1234567)?; // write a bogus checksum
    let out = close(&mut output)?;

    let mut input = BufferedChecksumIndexInput::new(Box::new(temp_input(&out)?));
    check_header(&mut input, "FooBar", 5, 5)?;
    assert_equals("this is the data", input.read_string()?.as_str());
    let mine = fake_exception();
    let expected = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        check_footer_throwing(&mut input, mine)
    });
    Assert::assert_true(message(&expected).contains("checksum failed"));
    let suppressed = expected.suppressed();
    assert_equals(&1, &suppressed.len());
    assert_equals("fake exception", message(&suppressed[0]).as_str());
    input.close()
}

/// Port of `TestCodecUtil.testSegmentHeaderLength`.
fn test_segment_header_length() -> Result<()> {
    let mut output = temp_output();
    write_index_header(&mut output, "FooBar", 5, &StringHelper::random_id(), "xyz")?;
    output.write_string("this is the data")?;
    let out = close(&mut output)?;

    let mut input = temp_input(&out)?;
    input.seek(index_header_length("FooBar", "xyz") as i64)?;
    assert_equals("this is the data", input.read_string()?.as_str());
    input.close()
}

/// Port of `TestCodecUtil.testWriteTooLongSuffix`.
fn test_write_too_long_suffix() -> Result<()> {
    let mut too_long = String::new();
    for _ in 0..256 {
        too_long.push('a');
    }
    let mut output = temp_output();
    expect_throws("java.lang.IllegalArgumentException", || {
        write_index_header(
            &mut output,
            "foobar",
            5,
            &StringHelper::random_id(),
            &too_long,
        )
    });
    Ok(())
}

/// Port of `TestCodecUtil.testWriteVeryLongSuffix`.
fn test_write_very_long_suffix() -> Result<()> {
    let mut just_long_enough = String::new();
    for _ in 0..255 {
        just_long_enough.push('a');
    }
    let mut output = temp_output();
    let id = StringHelper::random_id();
    write_index_header(&mut output, "foobar", 5, &id, &just_long_enough)?;
    let out = close(&mut output)?;

    let mut input = temp_input(&out)?;
    check_index_header(&mut input, "foobar", 5, 5, &id, &just_long_enough)?;
    assert_equals(&input.file_pointer(), &input.length());
    assert_equals(
        &input.file_pointer(),
        &(index_header_length("foobar", &just_long_enough) as i64),
    );
    input.close()
}

/// Port of `TestCodecUtil.testWriteNonAsciiSuffix`.
fn test_write_non_ascii_suffix() -> Result<()> {
    let mut output = temp_output();
    expect_throws("java.lang.IllegalArgumentException", || {
        write_index_header(
            &mut output,
            "foobar",
            5,
            &StringHelper::random_id(),
            "\u{1234}",
        )
    });
    Ok(())
}

/// Port of `TestCodecUtil.testReadBogusCRC`.
fn test_read_bogus_crc() -> Result<()> {
    let mut output = temp_output();
    write_be_long(&mut output, -1)?; // bad
    write_be_long(&mut output, 1i64 << 32)?; // bad
    write_be_long(&mut output, -(1i64 << 32))?; // bad);
    write_be_long(&mut output, (1i64 << 32) - 1)?; // ok
    let out = close(&mut output)?;
    let mut input = BufferedChecksumIndexInput::new(Box::new(temp_input(&out)?));
    // read 3 bogus values
    for _ in 0..3 {
        expect_throws("org.apache.lucene.index.CorruptIndexException", || {
            read_crc(&mut input)
        });
    }
    // good value
    read_crc(&mut input)?;
    Ok(())
}

/// The anonymous `IndexOutput("fake", "fake")` of `testWriteBogusCRC`: it
/// wraps `output` and answers `getChecksum()` with `fakeChecksum`.
struct FakeOutput {
    output: ByteBuffersIndexOutput,
    fake_checksum: Arc<AtomicI64>,
}

impl DataOutput for FakeOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.output.write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, length: usize) -> Result<()> {
        self.output.write_bytes(b, offset, length)
    }
}

impl IndexOutput for FakeOutput {
    fn close(&mut self) -> Result<()> {
        self.output.close()
    }

    fn file_pointer(&self) -> i64 {
        self.output.file_pointer()
    }

    fn checksum(&self) -> Result<i64> {
        Ok(self.fake_checksum.load(Ordering::SeqCst))
    }

    fn resource_description(&self) -> &str {
        "fake"
    }

    fn name(&self) -> &str {
        "fake"
    }
}

/// Port of `TestCodecUtil.testWriteBogusCRC`.
fn test_write_bogus_crc() -> Result<()> {
    let output = temp_output();
    let fake_checksum = Arc::new(AtomicI64::new(0));

    // wrap the index input where we control the checksum for mocking
    let mut fake_output = FakeOutput {
        output,
        fake_checksum: Arc::clone(&fake_checksum),
    };

    fake_checksum.store(-1, Ordering::SeqCst); // bad
    expect_throws("java.lang.IllegalStateException", || {
        write_crc(&mut fake_output)
    });

    fake_checksum.store(1i64 << 32, Ordering::SeqCst); // bad
    expect_throws("java.lang.IllegalStateException", || {
        write_crc(&mut fake_output)
    });

    fake_checksum.store(-(1i64 << 32), Ordering::SeqCst); // bad
    expect_throws("java.lang.IllegalStateException", || {
        write_crc(&mut fake_output)
    });

    fake_checksum.store((1i64 << 32) - 1, Ordering::SeqCst); // ok
    write_crc(&mut fake_output)
}

/// Port of `TestCodecUtil.testTruncatedFileThrowsCorruptIndexException`.
fn test_truncated_file_throws_corrupt_index_exception() -> Result<()> {
    let mut output = temp_output();
    let out = close(&mut output)?;

    let mut input = temp_input(&out)?;

    let e = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        checksum_entire_file(&mut input)
    });
    Assert::assert_true_msg(
        &message(&e),
        message(&e).contains(
            "misplaced codec footer (file truncated?): length=0 but footerLength==16 (resource",
        ),
    );

    let e = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
        retrieve_checksum(&mut input)
    });
    Assert::assert_true_msg(
        &message(&e),
        message(&e).contains(
            "misplaced codec footer (file truncated?): length=0 but footerLength==16 (resource",
        ),
    );
    Ok(())
}

/// Port of `TestCodecUtil.testRetrieveChecksum`.
fn test_retrieve_checksum() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    {
        let dir = directory.as_directory();
        {
            let mut out = dir.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
            out.write_byte(42)?;
            write_footer(&mut *out)?;
            out.close()?;
        }
        {
            let mut r#in = dir.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
            let length = r#in.length();
            retrieve_checksum_expected_length(&mut *r#in, length)?; // no exception

            let exception = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
                retrieve_checksum_expected_length(&mut *r#in, length - 1)
            });
            Assert::assert_true(message(&exception).contains("too long"));
            assert_equals(&0, &exception.suppressed().len());

            let exception = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
                retrieve_checksum_expected_length(&mut *r#in, length + 1)
            });
            Assert::assert_true(message(&exception).contains("truncated"));
            assert_equals(&0, &exception.suppressed().len());
            r#in.close()?;
        }

        {
            let mut out = dir.create_output("bar", &*DEFAULT_IO_CONTEXT)?;
            for i in 0..=footer_length() {
                out.write_byte(i as u8)?;
            }
            out.close()?;
        }
        {
            let mut r#in = dir.open_input("bar", &*DEFAULT_IO_CONTEXT)?;
            let length = r#in.length();
            let exception = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
                retrieve_checksum_expected_length(&mut *r#in, length)
            });
            Assert::assert_true(message(&exception).contains("codec footer mismatch"));
            assert_equals(&0, &exception.suppressed().len());

            let exception = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
                retrieve_checksum_expected_length(&mut *r#in, length - 1)
            });
            Assert::assert_true(message(&exception).contains("too long"));

            let exception = expect_throws("org.apache.lucene.index.CorruptIndexException", || {
                retrieve_checksum_expected_length(&mut *r#in, length + 1)
            });
            Assert::assert_true(message(&exception).contains("truncated"));
            r#in.close()?;
        }
    }
    directory.close()
}
