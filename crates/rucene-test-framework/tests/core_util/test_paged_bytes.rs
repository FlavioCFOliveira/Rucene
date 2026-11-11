//! Port of `lucene/core/src/test/org/apache/lucene/util/TestPagedBytes.java`.
//!
//! `PagedBytes.Reader.fillSlice(slice, pos, len)` points `slice` at the
//! bytes; Rucene's [`PagedBytesReader::fill_slice`] returns them.
//! `testOverflow` is `@Ignore // memory hole`: JUnit never runs it, so it is
//! translated below but not declared.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::store::{DataInput, DataOutput, DEFAULT_IO_CONTEXT};
use rucene::util::{Accountable, BytesRef, PagedBytes};
use rucene_test_framework::store::mock_directory_wrapper::Throttling;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::ram_usage_tester::{ModelBuilder, RamUsageTester};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestPagedBytes`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestPagedBytes",
        vec![
            test("testDataInputOutput", test_data_input_output),
            test("testDataInputOutput2", test_data_input_output2),
            test("testRamBytesUsed", test_ram_bytes_used),
        ],
    )
}

// Writes random byte/s to "normal" file in dir, then copies into PagedBytes and
// verifies with PagedBytes.Reader:
fn test_data_input_output() -> Result<()> {
    let random = random();
    let num_iters = LuceneTestCase::at_least(1);
    for _ in 0..num_iters {
        let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
            "testOverflow",
        ));
        if let Some(mock) = dir.as_mock() {
            mock.set_throttling(Throttling::Never);
        }
        let block_bits = TestUtil::next_int(&*random, 1, 20);
        let block_size = 1i32 << block_bits;
        let mut p = PagedBytes::new(block_bits as u32);
        let directory = dir.as_directory();
        let mut out = directory.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
        let num_bytes = if *TEST_NIGHTLY {
            TestUtil::next_int(&*LuceneTestCase::random(), 2, 10_000_000)
        } else {
            TestUtil::next_int(&*LuceneTestCase::random(), 2, 1_000_000)
        };

        let mut answer = vec![0u8; num_bytes as usize];
        random.next_bytes(&mut answer);
        let mut written = 0i32;
        while written < num_bytes {
            if random.next_int_bound(100) == 7 {
                out.write_byte(answer[written as usize])?;
                written += 1;
            } else {
                let chunk = random.next_int_bound(1000).min(num_bytes - written);
                out.write_bytes(&answer, written as usize, chunk as usize)?;
                written += chunk;
            }
        }

        out.close()?;
        let mut input = directory.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
        let mut r#in = input.clone_input()?;

        let length = input.length();
        p.copy_from_input(&mut *input, length)?;
        let reader = p.freeze(random.next_boolean())?;

        let mut verify = vec![0u8; num_bytes as usize];
        let mut read = 0i32;
        while read < num_bytes {
            if random.next_int_bound(100) == 7 {
                verify[read as usize] = r#in.read_byte()?;
                read += 1;
            } else {
                let chunk = random.next_int_bound(1000).min(num_bytes - read);
                r#in.read_bytes(Some(&mut verify), read as usize, chunk as usize)?;
                read += chunk;
            }
        }
        Assert::assert_true(answer == verify);

        for _ in 0..100 {
            let pos = random.next_int_bound(num_bytes - 1);
            assert_equals(&answer[pos as usize], &reader.get_byte(pos as i64));
            let len = random.next_int_bound((block_size + 1).min(num_bytes - pos));
            let slice = reader.fill_slice(pos as i64, len as usize);
            for byte_upto in 0..len as usize {
                assert_equals(&answer[pos as usize + byte_upto], &slice[byte_upto]);
            }
        }
        input.close()?;
        drop(directory);
        dir.close()?;
    }
    Ok(())
}

// Writes random byte/s into PagedBytes via .getDataOutput(), then verifies
// with PagedBytes.getDataInput():
fn test_data_input_output2() -> Result<()> {
    let random = random();
    let num_iters = LuceneTestCase::at_least(1);
    for _ in 0..num_iters {
        let block_bits = TestUtil::next_int(&*random, 1, 20);
        let block_size = 1i32 << block_bits;
        let mut p = PagedBytes::new(block_bits as u32);
        let num_bytes = if *TEST_NIGHTLY {
            TestUtil::next_int(&*LuceneTestCase::random(), 1, 10_000_000)
        } else {
            TestUtil::next_int(&*LuceneTestCase::random(), 1, 1_000_000)
        };

        let mut answer = vec![0u8; num_bytes as usize];
        LuceneTestCase::random().next_bytes(&mut answer);
        {
            let mut out = p.get_data_output()?;
            let mut written = 0i32;
            while written < num_bytes {
                if LuceneTestCase::random().next_int_bound(10) == 7 {
                    out.write_byte(answer[written as usize])?;
                    written += 1;
                } else {
                    let chunk = LuceneTestCase::random()
                        .next_int_bound(1000)
                        .min(num_bytes - written);
                    out.write_bytes(&answer, written as usize, chunk as usize)?;
                    written += chunk;
                }
            }
        }

        let reader = p.freeze(random.next_boolean())?;

        let mut r#in = p.get_data_input()?;

        let mut verify = vec![0u8; num_bytes as usize];
        let mut read = 0i32;
        while read < num_bytes {
            if LuceneTestCase::random().next_int_bound(10) == 7 {
                verify[read as usize] = r#in.read_byte()?;
                read += 1;
            } else {
                let chunk = LuceneTestCase::random()
                    .next_int_bound(1000)
                    .min(num_bytes - read);
                r#in.read_bytes(Some(&mut verify), read as usize, chunk as usize)?;
                read += chunk;
            }
        }
        Assert::assert_true(answer == verify);

        for _ in 0..100 {
            let pos = random.next_int_bound(num_bytes - 1);
            let len = random.next_int_bound((block_size + 1).min(num_bytes - pos));
            let slice = reader.fill_slice(pos as i64, len as usize);
            for byte_upto in 0..len as usize {
                assert_equals(&answer[pos as usize + byte_upto], &slice[byte_upto]);
            }
        }

        // test skipping
        let mut in2 = p.get_data_input()?;
        let max_skip_to = num_bytes - 1;
        let mut curr = 0;
        while curr < max_skip_to {
            let skip_to = TestUtil::next_int(&*random, curr, max_skip_to);
            let step = skip_to - curr;
            in2.skip_bytes(step as i64)?;
            assert_equals(&answer[skip_to as usize], &in2.read_byte()?);
            curr = skip_to + 1; // +1 for read byte
        }
    }
    Ok(())
}

// `@Ignore // memory hole`.
#[allow(dead_code)]
fn test_overflow() -> Result<()> {
    let mut dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("testOverflow"));
    if let Some(mock) = dir.as_mock() {
        mock.set_throttling(Throttling::Never);
    }
    let block_bits = TestUtil::next_int(&*random(), 14, 28);
    let block_size = 1i32 << block_bits;
    let mut arr =
        vec![0u8; TestUtil::next_int(&*random(), block_size / 2, block_size * 2) as usize];
    for i in 0..arr.len() {
        arr[i] = i as u8;
    }
    let num_bytes: i64 = (1i64 << 31) + TestUtil::next_int(&*random(), 1, block_size * 3) as i64;
    let mut p = PagedBytes::new(block_bits as u32);
    let directory = dir.as_directory();
    let mut out = directory.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
    let mut i: i64 = 0;
    while i < num_bytes {
        assert_equals(&i, &out.file_pointer());
        let len = (arr.len() as i64).min(num_bytes - i) as usize;
        out.write_bytes(&arr, 0, len)?;
        i += len as i64;
    }
    assert_equals(&num_bytes, &out.file_pointer());
    out.close()?;
    let mut r#in = directory.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
    p.copy_from_input(&mut *r#in, num_bytes)?;
    let reader = p.freeze(random().next_boolean())?;

    for offset in [
        0i64,
        i32::MAX as i64,
        num_bytes - 1,
        TestUtil::next_long(&*random(), 1, num_bytes - 2),
    ] {
        let b = BytesRef::new(reader.fill_slice(offset, 1).into_owned());
        assert_equals(
            &arr[(offset % arr.len() as i64) as usize],
            &b.bytes[b.offset],
        );
    }
    r#in.close()?;
    drop(directory);
    dir.close()
}

fn test_ram_bytes_used() -> Result<()> {
    let block_bits = TestUtil::next_int(&*random(), 4, 22);
    let mut b = PagedBytes::new(block_bits as u32);
    let total_bytes = random().next_int_bound(10000);
    let mut pointer: i64 = 0;
    while pointer < total_bytes as i64 {
        let bytes =
            LuceneTestCase::new_bytes_ref_str(&TestUtil::random_simple_string_max(&*random(), 10));
        pointer = b.copy_using_length_prefix(&bytes)?;
    }
    assert_equals(
        &RamUsageTester::ram_used(&*ModelBuilder::new().paged_bytes(&b)),
        &b.ram_bytes_used(),
    );
    let reader = b.freeze(random().next_boolean())?;
    assert_equals(
        &RamUsageTester::ram_used(&*ModelBuilder::new().paged_bytes(&b)),
        &b.ram_bytes_used(),
    );
    assert_equals(
        &RamUsageTester::ram_used(&*ModelBuilder::new().paged_bytes_reader(&reader)),
        &reader.ram_bytes_used(),
    );
    Ok(())
}
