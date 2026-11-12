//! Port of `lucene/core/src/test/org/apache/lucene/util/packed/TestDirectPacked.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene::util::extra::LongValues;
use rucene::util::packed::{shared_random_access_input, DirectReader, DirectWriter, PackedInts};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};

/// `TestDirectPacked`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testSimple", test_simple),
        test("testNotEnoughValues", test_not_enough_values),
        test("testRandom", test_random),
        test("testRandomWithOffset", test_random_with_offset),
        test("testRandomMerge", test_random_merge),
        test("testRandomMergeWithOffset", test_random_merge_with_offset),
    ];
    class("org.apache.lucene.util.packed.TestDirectPacked", methods)
}

/// `testSimple()`: simple encode/decode.
fn test_simple() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let bits_per_value = DirectWriter::bits_required(2)?;
    let mut output = d.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
    {
        let mut writer = DirectWriter::new(output.as_mut(), 5, bits_per_value)?;
        writer.add(1)?;
        writer.add(0)?;
        writer.add(2)?;
        writer.add(1)?;
        writer.add(2)?;
        writer.finish()?;
    }
    output.close()?;
    let mut input = d.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
    let reader = DirectReader::with_random_access(
        shared_random_access_input(input.random_access_slice(0, input.length())?),
        bits_per_value,
        0,
    )?;
    assert_equals(&1, &reader.get(0));
    assert_equals(&0, &reader.get(1));
    assert_equals(&2, &reader.get(2));
    assert_equals(&1, &reader.get(3));
    assert_equals(&2, &reader.get(4));
    input.close()?;
    dir.close()
}

/// `testNotEnoughValues()`: test exception is delivered if you add the
/// wrong number of values.
fn test_not_enough_values() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let bits_per_value = DirectWriter::bits_required(2)?;
    let mut output = d.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
    {
        let mut writer = DirectWriter::new(output.as_mut(), 5, bits_per_value)?;
        writer.add(1)?;
        writer.add(0)?;
        writer.add(2)?;
        writer.add(1)?;
        let expected = expect_throws("java.lang.IllegalStateException", || writer.finish());
        Assert::assert_true(
            expected
                .message()
                .unwrap_or("")
                .starts_with("Wrong number of values added"),
        );
    }
    output.close()?;
    dir.close()
}

fn test_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    for bpv in 1..=64 {
        do_test_bpv(&d, bpv, 0, false)?;
    }
    dir.close()
}

fn test_random_with_offset() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let offset = TestUtil::next_int(&*random(), 1, 100) as i64;
    for bpv in 1..=64 {
        do_test_bpv(&d, bpv, offset, false)?;
    }
    dir.close()
}

fn test_random_merge() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    for bpv in 1..=64 {
        do_test_bpv(&d, bpv, 0, true)?;
    }
    dir.close()
}

fn test_random_merge_with_offset() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let offset = TestUtil::next_int(&*random(), 1, 100) as i64;
    for bpv in 1..=64 {
        do_test_bpv(&d, bpv, offset, true)?;
    }
    dir.close()
}

/// `doTestBpv(directory, bpv, offset, merge)`.
fn do_test_bpv(directory: &Arc<dyn Directory>, bpv: i32, offset: i64, merge: bool) -> Result<()> {
    let random = random();
    let num_iters = if *TEST_NIGHTLY { 100 } else { 10 };
    for i in 0..num_iters {
        let original = random_longs(&*random, bpv);
        let bits_required = if bpv == 64 {
            64
        } else {
            DirectWriter::bits_required(1i64 << (bpv - 1))?
        };
        let name = format!("bpv{bpv}_{i}");
        let mut output = directory.create_output(&name, &*DEFAULT_IO_CONTEXT)?;
        for _ in 0..offset {
            output.write_byte(random.next_int() as u8)?;
        }
        {
            let mut writer =
                DirectWriter::new(output.as_mut(), original.len() as i64, bits_required)?;
            for &v in &original {
                writer.add(v)?;
            }
            writer.finish()?;
        }
        output.close()?;
        let mut input = directory.open_input(&name, &*DEFAULT_IO_CONTEXT)?;
        assert_equals(
            &(input.length() - offset),
            &DirectWriter::bytes_required(original.len() as i64, bits_required)?,
        );
        let slice = shared_random_access_input(input.random_access_slice(0, input.length())?);
        let reader: Box<dyn LongValues> = if merge {
            Box::new(DirectReader::get_merge_instance(
                slice,
                bits_required,
                offset,
                original.len() as i64,
            ))
        } else {
            Box::new(DirectReader::with_random_access(
                slice,
                bits_required,
                offset,
            )?)
        };
        for (j, &v) in original.iter().enumerate() {
            assert_equals_msg(Some(&format!("bpv={bpv}")), &v, &reader.get(j as i64));
        }
        input.close()?;
    }
    Ok(())
}

/// `randomLongs(random, bpv)`.
fn random_longs(random: &dyn Random, bpv: i32) -> Vec<i64> {
    let amount = random.next_int_bound(5000) as usize;
    let mut longs = vec![0i64; amount];
    let max = PackedInts::max_value(bpv);
    for l in longs.iter_mut() {
        *l = RandomNumbers::random_long_between(random, 0, max);
    }
    longs
}
