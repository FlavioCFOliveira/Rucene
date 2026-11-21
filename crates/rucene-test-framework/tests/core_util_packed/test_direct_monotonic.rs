//! Port of `lucene/core/src/test/org/apache/lucene/util/packed/TestDirectMonotonic.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::store::{
    ByteBuffersDataOutput, ByteBuffersIndexOutput, Directory, IndexInput, DEFAULT_IO_CONTEXT,
    READONCE_IO_CONTEXT,
};
use rucene::util::extra::LongValues;
use rucene::util::packed::{
    shared_random_access_input, DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter,
};
use rucene::util::ArrayUtil;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestDirectMonotonic`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testValidation", test_validation),
        test("testEmpty", test_empty),
        test("testSimple", test_simple),
        test("testConstantSlope", test_constant_slope),
        test(
            "testZeroValuesSmallBlobShift",
            test_zero_values_small_blob_shift,
        ),
        test("testRandom", test_random),
        test("testRandomMerging", test_random_merging),
        test("testMonotonicBinarySearch", test_monotonic_binary_search),
        test(
            "testMonotonicBinarySearchRandom",
            test_monotonic_binary_search_random,
        ),
    ];
    class("org.apache.lucene.util.packed.TestDirectMonotonic", methods)
}

const IAE: &str = "java.lang.IllegalArgumentException";

/// An output nothing is written to.
///
/// **Divergence from Lucene 10.5.0.** Java passes `null` outputs to
/// `DirectMonotonicWriter.getInstance`, which validates its arguments before
/// touching them; Rucene's writer takes the outputs as references, so the
/// port passes in-memory outputs that are never written.
fn unused_output() -> ByteBuffersIndexOutput {
    ByteBuffersIndexOutput::new(ByteBuffersDataOutput::new(), "unused", "unused")
}

fn test_validation() -> Result<()> {
    let mut e = expect_throws(IAE, || {
        let (mut m, mut d) = (unused_output(), unused_output());
        DirectMonotonicWriter::new(&mut m, &mut d, -1, 10).map(|_| ())
    });
    assert_equals(&Some("numValues can't be negative, got -1"), &e.message());

    e = expect_throws(IAE, || {
        let (mut m, mut d) = (unused_output(), unused_output());
        DirectMonotonicWriter::new(&mut m, &mut d, 10, 1).map(|_| ())
    });
    assert_equals(&Some("blockShift must be in [2-22], got 1"), &e.message());

    e = expect_throws(IAE, || {
        let (mut m, mut d) = (unused_output(), unused_output());
        DirectMonotonicWriter::new(&mut m, &mut d, 1i64 << 40, 5).map(|_| ())
    });
    let expected = format!(
        "blockShift is too low for the provided number of values: blockShift=5, numValues=1099511627776, MAX_ARRAY_LENGTH={}",
        ArrayUtil::MAX_ARRAY_LENGTH
    );
    assert_equals(&Some(expected.as_str()), &e.message());
    Ok(())
}

/// `try (IndexOutput metaOut = …; IndexOutput dataOut = …) { write }`:
/// writes `values` and returns the data length.
fn write(
    dir: &Arc<dyn Directory>,
    num_values: i64,
    block_shift: i32,
    values: &[i64],
) -> Result<i64> {
    let mut meta_out = dir.create_output("meta", &*DEFAULT_IO_CONTEXT)?;
    let mut data_out = dir.create_output("data", &*DEFAULT_IO_CONTEXT)?;
    {
        let mut w = DirectMonotonicWriter::new(
            meta_out.as_mut(),
            data_out.as_mut(),
            num_values,
            block_shift,
        )?;
        for &v in values {
            w.add(v)?;
        }
        w.finish()?;
    }
    let data_length = data_out.file_pointer();
    data_out.close()?;
    meta_out.close()?;
    Ok(data_length)
}

/// `DirectMonotonicReader.getInstance(meta, dataIn.randomAccessSlice(0,
/// dataLength), merging)`.
fn get_instance(
    meta: DirectMonotonicMeta,
    data_in: &dyn IndexInput,
    data_length: i64,
    merging: bool,
) -> Result<DirectMonotonicReader> {
    DirectMonotonicReader::with_random_access_merging(
        meta,
        shared_random_access_input(data_in.random_access_slice(0, data_length)?),
        merging,
    )
}

fn test_empty() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let block_shift = TestUtil::next_int(
        &*random(),
        DirectMonotonicWriter::MIN_BLOCK_SHIFT,
        DirectMonotonicWriter::MAX_BLOCK_SHIFT,
    );

    let data_length = write(&d, 0, block_shift, &[])?;

    {
        let mut meta_in = d.open_input("meta", &*READONCE_IO_CONTEXT)?;
        let mut data_in = d.open_input("data", &*DEFAULT_IO_CONTEXT)?;
        let meta = DirectMonotonicMeta::load(meta_in.as_mut(), 0, block_shift)?;
        get_instance(meta, data_in.as_ref(), data_length, false)?;
        // no exception
        data_in.close()?;
        meta_in.close()?;
    }

    dir.close()
}

fn test_simple() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let block_shift = 2;

    let actual_values: Vec<i64> = vec![1, 2, 5, 7, 8, 100];
    let num_values = actual_values.len();

    let data_length = write(&d, num_values as i64, block_shift, &actual_values)?;

    {
        let mut meta_in = d.open_input("meta", &*READONCE_IO_CONTEXT)?;
        let mut data_in = d.open_input("data", &*DEFAULT_IO_CONTEXT)?;
        let meta = DirectMonotonicMeta::load(meta_in.as_mut(), num_values as i64, block_shift)?;
        let values = get_instance(meta, data_in.as_ref(), data_length, false)?;
        for (i, &expected) in actual_values.iter().enumerate() {
            let v = values.get(i as i64);
            assert_equals(&expected, &v);
        }
        data_in.close()?;
        meta_in.close()?;
    }

    dir.close()
}

fn test_constant_slope() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let block_shift = TestUtil::next_int(
        &*random(),
        DirectMonotonicWriter::MIN_BLOCK_SHIFT,
        DirectMonotonicWriter::MAX_BLOCK_SHIFT,
    );
    let num_values = TestUtil::next_int(&*random(), 1, 1 << 20);
    let min = random().next_long();
    let bound = 1 << random().next_int_bound(20);
    let inc = random().next_int_bound(bound) as i64;

    let mut actual_values = Vec::new();
    for i in 0..num_values as i64 {
        actual_values.push(min.wrapping_add(inc.wrapping_mul(i)));
    }

    let data_length = write(&d, num_values as i64, block_shift, &actual_values)?;

    {
        let mut meta_in = d.open_input("meta", &*READONCE_IO_CONTEXT)?;
        let mut data_in = d.open_input("data", &*DEFAULT_IO_CONTEXT)?;
        let meta = DirectMonotonicMeta::load(meta_in.as_mut(), num_values as i64, block_shift)?;
        let values = get_instance(meta, data_in.as_ref(), data_length, false)?;
        for (i, &expected) in actual_values.iter().enumerate() {
            assert_equals(&expected, &values.get(i as i64));
        }
        assert_equals(&0, &data_in.file_pointer()?);
        data_in.close()?;
        meta_in.close()?;
    }

    dir.close()
}

fn test_zero_values_small_blob_shift() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let num_values = TestUtil::next_int(&*random(), 8, 1 << 20);
    // use blockShift < log2(numValues)
    let log2 = ((num_values as f64).ln() / 2f64.ln()).round() as i64;
    let block_shift = TestUtil::next_int(
        &*random(),
        DirectMonotonicWriter::MIN_BLOCK_SHIFT,
        i32::try_from(log2).expect("Math.toIntExact") - 1,
    );

    let zeros = vec![0i64; num_values as usize];
    let data_length = write(&d, num_values as i64, block_shift, &zeros)?;

    {
        let mut meta_in = d.open_input("meta", &*READONCE_IO_CONTEXT)?;
        let mut data_in = d.open_input("data", &*DEFAULT_IO_CONTEXT)?;
        let meta = DirectMonotonicMeta::load(meta_in.as_mut(), num_values as i64, block_shift)?;
        assert_equals(&meta_in.length(), &meta_in.file_pointer()?);
        // read meta again and assert singleton Meta#SINGLE_ZERO_BLOCK instance is read every time
        meta_in.seek(0)?;
        // assertSame(meta, loadMeta(…)): see `assert_same_meta`.
        let again = DirectMonotonicMeta::load(meta_in.as_mut(), num_values as i64, block_shift)?;
        assert_same_meta(&meta, &again);
        let values = get_instance(meta, data_in.as_ref(), data_length, false)?;
        for i in 0..num_values as i64 {
            assert_equals(&0, &values.get(i));
        }
        assert_equals(&0, &data_in.file_pointer()?);
        data_in.close()?;
        meta_in.close()?;
    }

    dir.close()
}

/// `assertSame(meta, DirectMonotonicReader.loadMeta(…))`.
///
/// **Divergence from Lucene 10.5.0.** Java asserts that `loadMeta` returns
/// the singleton `Meta.SINGLE_ZERO_BLOCK` (object identity); Rucene's
/// `DirectMonotonicMeta::load` returns a value, so the port asserts that both
/// loads equal `DirectMonotonicMeta::single_zero_block()` (compared through
/// their `Debug` rendering, the type having no `PartialEq`).
fn assert_same_meta(meta: &DirectMonotonicMeta, again: &DirectMonotonicMeta) {
    let single = format!("{:?}", DirectMonotonicMeta::single_zero_block());
    assert_equals(&single, &format!("{meta:?}"));
    assert_equals(&single, &format!("{again:?}"));
}

fn test_random() -> Result<()> {
    do_test_random(false)
}

fn test_random_merging() -> Result<()> {
    do_test_random(true)
}

/// `doTestRandom(merging)`.
fn do_test_random(merging: bool) -> Result<()> {
    let random = random();
    let iters = LuceneTestCase::at_least_with(&*random, 3);
    for _ in 0..iters {
        let dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();
        let block_shift = TestUtil::next_int(
            &*random,
            DirectMonotonicWriter::MIN_BLOCK_SHIFT,
            DirectMonotonicWriter::MAX_BLOCK_SHIFT,
        );
        let max_num_values = 1 << 20;
        let num_values = if random.next_boolean() {
            // random number
            TestUtil::next_int(&*random, 1, max_num_values)
        } else {
            // multiple of the block size
            let num_blocks = TestUtil::next_int(&*random, 0, max_num_values >> block_shift);
            TestUtil::next_int(&*random, 0, num_blocks) << block_shift
        };
        let mut actual_values = Vec::new();
        let mut previous = random.next_long();
        if num_values > 0 {
            actual_values.push(previous);
        }
        for _ in 1..num_values {
            let bound = 1 << random.next_int_bound(20);
            previous = previous.wrapping_add(random.next_int_bound(bound) as i64);
            actual_values.push(previous);
        }

        let data_length = write(&d, num_values as i64, block_shift, &actual_values)?;

        {
            let mut meta_in = d.open_input("meta", &*READONCE_IO_CONTEXT)?;
            let mut data_in = d.open_input("data", &*DEFAULT_IO_CONTEXT)?;
            let meta = DirectMonotonicMeta::load(meta_in.as_mut(), num_values as i64, block_shift)?;
            let values = get_instance(meta, data_in.as_ref(), data_length, merging)?;
            for (i, &expected) in actual_values.iter().enumerate() {
                assert_equals(&expected, &values.get(i as i64));
            }
            data_in.close()?;
            meta_in.close()?;
        }

        dir.close()?;
    }
    Ok(())
}

fn test_monotonic_binary_search() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let result = do_test_monotonic_binary_search_against_long_array(
        &d,
        &[4, 7, 8, 10, 19, 30, 55, 78, 100],
        2,
    );
    let closed = dir.close();
    result.and(closed)
}

fn test_monotonic_binary_search_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let result = (|| -> Result<()> {
        let iters = LuceneTestCase::at_least(100);
        for _ in 0..iters {
            let bound = 1 << random().next_int_bound(14);
            let array_length = random().next_int_bound(bound) as usize;
            let mut array = vec![0i64; array_length];
            let base = random().next_long();
            let bpv = TestUtil::next_int(&*random(), 4, 61);
            for v in array.iter_mut() {
                *v = base.wrapping_add(TestUtil::next_long(&*random(), 0, (1i64 << bpv) - 1));
            }
            array.sort();
            let block_shift = TestUtil::next_int(&*random(), 2, 10);
            do_test_monotonic_binary_search_against_long_array(&d, &array, block_shift)?;
        }
        Ok(())
    })();
    let closed = dir.close();
    result.and(closed)
}

/// `doTestMonotonicBinarySearchAgainstLongArray(dir, array, blockShift)`.
fn do_test_monotonic_binary_search_against_long_array(
    dir: &Arc<dyn Directory>,
    array: &[i64],
    block_shift: i32,
) -> Result<()> {
    write(dir, array.len() as i64, block_shift, array)?;

    {
        let mut meta_in = dir.open_input("meta", &*READONCE_IO_CONTEXT)?;
        let mut data_in = dir.open_input("data", &*DEFAULT_IO_CONTEXT)?;
        let meta = DirectMonotonicMeta::load(meta_in.as_mut(), array.len() as i64, block_shift)?;
        let reader = DirectMonotonicReader::with_random_access(
            meta,
            shared_random_access_input(data_in.random_access_slice(0, dir.file_length("data")?)?),
        )?;

        let len = array.len() as i64;
        if array.is_empty() {
            assert_equals(&-1, &reader.binary_search(0, len, 42)?);
        } else {
            for &v in array {
                let index = reader.binary_search(0, len, v)?;
                Assert::assert_true(index >= 0);
                Assert::assert_true(index < len);
                assert_equals(&v, &array[index as usize]);
            }
            if array[0] != i64::MIN {
                assert_equals(&-1, &reader.binary_search(0, len, array[0] - 1)?);
            }
            if array[array.len() - 1] != i64::MAX {
                assert_equals(
                    &(-1 - len),
                    &reader.binary_search(0, len, array[array.len() - 1] + 1)?,
                );
            }
            for i in 0..array.len().saturating_sub(2) {
                if array[i] + 1 < array[i + 1] {
                    let intermediate = if random().next_boolean() {
                        array[i] + 1
                    } else {
                        array[i + 1] - 1
                    };
                    let index = reader.binary_search(0, len, intermediate)?;
                    Assert::assert_true(index < 0);
                    let insertion_point =
                        i32::try_from(-1 - index).expect("Math.toIntExact") as usize;
                    Assert::assert_true(insertion_point > 0);
                    Assert::assert_true(insertion_point < array.len());
                    Assert::assert_true(array[insertion_point] > intermediate);
                    Assert::assert_true(array[insertion_point - 1] < intermediate);
                }
            }
        }
        data_in.close()?;
        meta_in.close()?;
    }
    dir.delete_file("meta")?;
    dir.delete_file("data")
}
