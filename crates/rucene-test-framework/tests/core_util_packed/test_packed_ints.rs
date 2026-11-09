//! Port of `lucene/core/src/test/org/apache/lucene/util/packed/TestPackedInts.java`.
//!
//! Adaptations (Rust has no counterpart for the Java form; each is local):
//!
//! * `getClass().getSimpleName()` in messages: [`create_packed_ints`] pairs
//!   each mutable with its Java simple name.
//! * `PagedMutable.grow` / `PagedGrowableWriter.grow` consume `self` in
//!   Rucene; the values the Java test reads back from `writer` after `grow`
//!   are read before it.
//! * The writer of `testPackedInts` borrows the output, so `startFp` is read
//!   just before the writer is created (the constructor writes nothing).
//! * `RamUsageTester.ramUsed` over `GrowableWriter`, `PagedGrowableWriter`,
//!   `PagedMutable` and the `PackedLongValues` builders and values: Rucene has
//!   no `RamObject` model of them ([`ram_used_unmodelled`]); the
//!   `IgnoreNullReaderSingletonAccumulator` those measurements use is not
//!   translated for that reason.
//! * `buf.add(…)` after `buf.build()` (`IllegalStateException` in Java) is
//!   not expressible: Rucene's `build` consumes the builder.
//!
//! `testIntOverflow` (`@Ignore("See LUCENE-4488")`) and
//! `testPagedGrowableWriterOverflow` (`@Ignore`) are not declared, as JUnit
//! never runs them.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::store::{ByteArrayDataInput, DataInput, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT};
use rucene::util::extra::LongValues;
use rucene::util::packed::bulk_operation::{get_decoder, get_encoder};
use rucene::util::packed::packed_long_values::PackedLongValuesBuilderOps;
use rucene::util::packed::paged_mutable::{
    AbstractPagedMutableOps, PagedGrowableWriter, PagedMutable,
};
use rucene::util::packed::{
    AbstractBlockPackedWriterOps, BlockPackedReaderIterator, BlockPackedWriter, Format,
    GrowableWriter, MonotonicBlockPackedReader, MonotonicBlockPackedWriter, NullReader, Packed64,
    Packed64SingleBlock, PackedDataInput, PackedDataOutput, PackedInts, PackedIntsMutable,
    PackedIntsReader, PackedIntsReaderIterator, PackedIntsWriter, PackedLongValues,
};
use rucene::util::{Accountable, ArrayUtil};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{Nightly, TEST_NIGHTLY};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_array_equals_msg, assert_equals, assert_equals_msg, class, ram_used_unmodelled, random,
    test, verbose,
};

/// `TestPackedInts`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.packed.TestPackedInts",
        vec![
            test("testByteCount", test_byte_count),
            test("testBitsRequired", test_bits_required),
            test("testBitsRequiredInt", test_bits_required_int),
            test("testMaxValues", test_max_values),
            test("testPackedInts", test_packed_ints),
            test("testEndPointer", test_end_pointer),
            test("testControlledEquality", test_controlled_equality),
            test("testRandomBulkCopy", test_random_bulk_copy),
            test("testRandomEquality", test_random_equality),
            test("testSecondaryBlockChange", test_secondary_block_change),
            test("testFill", test_fill),
            test("testPackedIntsNull", test_packed_ints_null),
            test("testBulkGet", test_bulk_get),
            test("testBulkSet", test_bulk_set),
            test("testCopy", test_copy),
            test("testGrowableWriter", test_growable_writer),
            test("testPagedGrowableWriter", test_paged_growable_writer),
            test("testPagedMutable", test_paged_mutable),
            test("testEncodeDecode", test_encode_decode),
            test(
                "testPackedLongValuesOnZeros",
                test_packed_long_values_on_zeros,
            ),
            test("testPackedLongValues", test_packed_long_values),
            test(
                "testPackedLongValuesAddRepeat",
                test_packed_long_values_add_repeat,
            ),
            test("testPackedInputOutput", test_packed_input_output),
            test(
                "testBlockPackedReaderWriter",
                test_block_packed_reader_writer,
            ),
            test(
                "testMonotonicBlockPackedReaderWriter",
                test_monotonic_block_packed_reader_writer,
            ),
            test("testBlockReaderOverflow", test_block_reader_overflow).annotate(Nightly),
        ],
    )
}

/// `PackedInts.Format.values()`.
const FORMATS: [Format; 2] = [Format::Packed, Format::PackedSingleBlock];

fn test_byte_count() -> Result<()> {
    let iters = LuceneTestCase::at_least(3);
    for _ in 0..iters {
        let value_count = RandomNumbers::random_int_between(&*random(), 1, i32::MAX);
        for format in FORMATS {
            for bpv in 1..=64 {
                let byte_count =
                    format.byte_count(PackedInts::VERSION_CURRENT, value_count, bpv)?;
                let msg = format!(
                    "format={format:?}, byteCount={byte_count}, valueCount={value_count}, bpv={bpv}"
                );
                Assert::assert_true_msg(&msg, byte_count * 8 >= value_count as i64 * bpv as i64);
                if format == Format::Packed {
                    Assert::assert_true_msg(
                        &msg,
                        (byte_count - 1) * 8 < value_count as i64 * bpv as i64,
                    );
                }
            }
        }
    }
    Ok(())
}

fn test_bits_required() -> Result<()> {
    assert_equals(&61, &PackedInts::bits_required(2f64.powi(61) as i64 - 1)?);
    assert_equals(&61, &PackedInts::bits_required(0x1FFFFFFFFFFFFFFF)?);
    assert_equals(&62, &PackedInts::bits_required(0x3FFFFFFFFFFFFFFF)?);
    assert_equals(&63, &PackedInts::bits_required(0x7FFFFFFFFFFFFFFF)?);
    assert_equals(&64, &PackedInts::unsigned_bits_required(-1));
    assert_equals(&64, &PackedInts::unsigned_bits_required(i64::MIN));
    assert_equals(&1, &PackedInts::bits_required(0)?);
    Ok(())
}

fn test_bits_required_int() -> Result<()> {
    assert_equals(
        &29,
        &PackedInts::bits_required_i32(2f64.powi(29) as i32 - 1)?,
    );
    assert_equals(&30, &PackedInts::bits_required_i32(0x3FFFFFFF)?);
    assert_equals(&31, &PackedInts::bits_required_i32(0x7FFFFFFF)?);
    assert_equals(&32, &PackedInts::unsigned_bits_required_i32(-1));
    assert_equals(&32, &PackedInts::unsigned_bits_required_i32(i32::MIN));
    assert_equals(&1, &PackedInts::bits_required_i32(0)?);
    Ok(())
}

fn test_max_values() -> Result<()> {
    assert_equals_msg(Some("1 bit -> max == 1"), &1, &PackedInts::max_value(1));
    assert_equals_msg(Some("2 bit -> max == 3"), &3, &PackedInts::max_value(2));
    assert_equals_msg(Some("8 bit -> max == 255"), &255, &PackedInts::max_value(8));
    assert_equals_msg(
        Some("63 bit -> max == Long.MAX_VALUE"),
        &i64::MAX,
        &PackedInts::max_value(63),
    );
    assert_equals_msg(
        Some("64 bit -> max == Long.MAX_VALUE (same as for 63 bit)"),
        &i64::MAX,
        &PackedInts::max_value(64),
    );
    Ok(())
}

fn test_packed_ints() -> Result<()> {
    let num = LuceneTestCase::at_least(3);
    for _ in 0..num {
        for nbits in 1..=64 {
            let max_value = PackedInts::max_value(nbits);
            let value_count = TestUtil::next_int(&*random(), 1, 600);
            let buffer_size = if random().next_boolean() {
                TestUtil::next_int(&*random(), 0, 48)
            } else {
                TestUtil::next_int(&*random(), 0, 4096)
            };
            let mut d = LuceneTestCase::new_directory();
            let dir = d.as_directory();

            let mut out =
                dir.create_output("out.bin", &*LuceneTestCase::new_io_context(&*random()))?;
            let mem = random().next_int_bound(2 * PackedInts::DEFAULT_BUFFER_SIZE as i32);
            let start_fp = out.file_pointer();
            let mut values = vec![0i64; value_count as usize];
            let (format, bits_per_value) = {
                let mut w = PackedInts::get_writer_no_header(
                    out.as_mut(),
                    Format::Packed,
                    value_count,
                    nbits,
                    mem,
                )?;

                let actual_value_count = if random().next_boolean() {
                    value_count
                } else {
                    TestUtil::next_int(&*random(), 0, value_count)
                };
                for i in 0..actual_value_count as usize {
                    values[i] = if nbits == 64 {
                        random().next_long()
                    } else {
                        TestUtil::next_long(&*random(), 0, max_value)
                    };
                    w.add(values[i])?;
                }
                w.finish()?;
                (w.format(), w.bits_per_value())
            };
            let fp = out.file_pointer();
            out.close()?;

            // packed writers should only write longs
            let bytes =
                format.byte_count(PackedInts::VERSION_CURRENT, value_count, bits_per_value)?;
            assert_equals(&bytes, &(fp - start_fp));

            {
                // test reader iterator next
                let mut r#in =
                    dir.open_input("out.bin", &*LuceneTestCase::new_io_context(&*random()))?;
                {
                    let mut r = PackedInts::get_reader_iterator_no_header(
                        r#in.as_mut(),
                        Format::Packed,
                        PackedInts::VERSION_CURRENT,
                        value_count,
                        nbits,
                        buffer_size,
                    )?;
                    for i in 0..value_count as usize {
                        assert_equals_msg(
                            Some(&format!(
                                "index={i} valueCount={value_count} nbits={nbits} for PackedReaderIterator"
                            )),
                            &values[i],
                            &r.next()?,
                        );
                        assert_equals(&(i as i32), &r.ord());
                    }
                }
                assert_equals(&fp, &r#in.file_pointer());
                r#in.close()?;
            }

            {
                // test reader iterator bulk next
                let mut r#in =
                    dir.open_input("out.bin", &*LuceneTestCase::new_io_context(&*random()))?;
                {
                    let mut r = PackedInts::get_reader_iterator_no_header(
                        r#in.as_mut(),
                        Format::Packed,
                        PackedInts::VERSION_CURRENT,
                        value_count,
                        nbits,
                        buffer_size,
                    )?;
                    let mut i = 0usize;
                    while i < value_count as usize {
                        let count = TestUtil::next_int(&*random(), 1, 95) as usize;
                        let next = r.next_batch(count)?;
                        for k in 0..next.len() {
                            assert_equals_msg(
                                Some(&format!(
                                    "index={i} valueCount={value_count} nbits={nbits} for PackedReaderIterator"
                                )),
                                &values[i + k],
                                &next[k],
                            );
                        }
                        i += next.len();
                    }
                }
                assert_equals(&fp, &r#in.file_pointer());
                r#in.close()?;
            }
            drop(dir);
            d.close()?;
        }
    }
    Ok(())
}

fn test_end_pointer() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let dir = d.as_directory();
    let value_count = RandomNumbers::random_int_between(&*random(), 1, 1000);
    let mut out = dir.create_output("tests.bin", &*LuceneTestCase::new_io_context(&*random()))?;
    for _ in 0..value_count {
        out.write_long(0)?;
    }
    out.close()?;
    let mut r#in = dir.open_input("tests.bin", &*LuceneTestCase::new_io_context(&*random()))?;
    for version in PackedInts::VERSION_START..=PackedInts::VERSION_CURRENT {
        for bpv in 1..=64 {
            for format in FORMATS {
                if !format.is_supported(bpv) {
                    continue;
                }
                let byte_count = format.byte_count(version, value_count, bpv)?;
                let msg = format!(
                    "format={format:?},version={version},valueCount={value_count},bpv={bpv}"
                );

                // test iterator
                r#in.seek(0)?;
                {
                    let mut it = PackedInts::get_reader_iterator_no_header(
                        r#in.as_mut(),
                        format,
                        version,
                        value_count,
                        bpv,
                        RandomNumbers::random_int_between(&*random(), 1, 1 << 16),
                    )?;
                    for _ in 0..value_count {
                        it.next()?;
                    }
                }
                assert_equals_msg(Some(&msg), &byte_count, &r#in.file_pointer());
            }
        }
    }
    r#in.close()?;
    drop(dir);
    d.close()
}

fn test_controlled_equality() -> Result<()> {
    const VALUE_COUNT: i32 = 255;
    const BITS_PER_VALUE: i32 = 8;

    let mut packed_ints = create_packed_ints(VALUE_COUNT, BITS_PER_VALUE)?;
    for (_, packed_int) in packed_ints.iter_mut() {
        for i in 0..packed_int.size() {
            packed_int.set(i, (i + 1) as i64);
        }
    }
    assert_list_equality(&packed_ints);
    Ok(())
}

fn test_random_bulk_copy() -> Result<()> {
    let random = random();
    let num_iters = LuceneTestCase::at_least_with(&*random, 3);
    for iter in 0..num_iters {
        if verbose() {
            println!("\nTEST: iter={iter}");
        }
        let value_count = if *TEST_NIGHTLY {
            LuceneTestCase::at_least_with(&*random, 100000)
        } else {
            LuceneTestCase::at_least_with(&*random, 10000)
        };
        let mut bits1 = TestUtil::next_int(&*random, 1, 64);
        let mut bits2 = TestUtil::next_int(&*random, 1, 64);
        if bits1 > bits2 {
            std::mem::swap(&mut bits1, &mut bits2);
        }
        if verbose() {
            println!("  valueCount={value_count} bits1={bits1} bits2={bits2}");
        }

        let mut packed1 = PackedInts::get_mutable(value_count, bits1, PackedInts::COMPACT)?;
        let mut packed2 = PackedInts::get_mutable(value_count, bits2, PackedInts::COMPACT)?;

        let max_value = PackedInts::max_value(bits1);
        for i in 0..value_count {
            let val = TestUtil::next_long(&*random, 0, max_value);
            packed1.set(i, val);
            packed2.set(i, val);
        }

        let mut buffer = vec![0i64; value_count as usize];

        // Copy random slice over, 20 times:
        for _ in 0..20 {
            let start = random.next_int_bound(value_count - 1);
            let len = TestUtil::next_int(&*random, 1, value_count - start);
            if verbose() {
                println!("  copy {len} values @ {start}");
            }
            let offset = if len == value_count {
                0
            } else {
                random.next_int_bound(value_count - len)
            };
            if random.next_boolean() {
                let got = packed1.get_bulk(start, &mut buffer, offset as usize, len as usize);
                Assert::assert_true(got <= len);
                let sot = packed2.set_bulk(start, &buffer, offset as usize, got as usize);
                Assert::assert_true(sot <= got);
            } else {
                PackedInts::copy(
                    packed1.as_ref(),
                    offset,
                    packed2.as_mut(),
                    offset,
                    len,
                    random.next_int_bound(10 * len),
                );
            }
        }

        for i in 0..value_count {
            assert_equals_msg(
                Some(&format!("value {i}")),
                &packed1.get(i),
                &packed2.get(i),
            );
        }
    }
    Ok(())
}

fn test_random_equality() -> Result<()> {
    let num_iters = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(2)
    } else {
        1
    };
    for _ in 0..num_iters {
        let value_count = TestUtil::next_int(&*random(), 1, 300);

        for bits_per_value in 1..=64 {
            assert_random_equality(value_count, bits_per_value, random().next_long())?;
        }
    }
    Ok(())
}

fn assert_random_equality(value_count: i32, bits_per_value: i32, random_seed: i64) -> Result<()> {
    let mut packed_ints = create_packed_ints(value_count, bits_per_value)?;
    for (name, packed_int) in packed_ints.iter_mut() {
        // try { fill(...) } catch (Exception e) { fail(...) }: `fill` raises
        // no checked exception in the port.
        let filled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fill(name, packed_int.as_mut(), bits_per_value, random_seed)
        }));
        if filled.is_err() {
            Assert::fail(&format!(
                "Exception while filling {name}: valueCount={value_count}, bitsPerValue={bits_per_value}"
            ));
        }
    }
    assert_list_equality(&packed_ints);
    Ok(())
}

/// A `PackedInts.Mutable` with its Java `getClass().getSimpleName()`.
type NamedMutable = (String, Box<dyn PackedIntsMutable>);

/// `createPackedInts(int valueCount, int bitsPerValue)`.
fn create_packed_ints(value_count: i32, bits_per_value: i32) -> Result<Vec<NamedMutable>> {
    let mut packed_ints: Vec<NamedMutable> = Vec::new();
    packed_ints.push((
        "Packed64".to_string(),
        Box::new(Packed64::new(value_count, bits_per_value)?),
    ));
    for bpv in bits_per_value..=Packed64SingleBlock::MAX_SUPPORTED_BITS_PER_VALUE {
        if Packed64SingleBlock::is_supported(bpv) {
            packed_ints.push((
                format!("Packed64SingleBlock{bpv}"),
                Box::new(Packed64SingleBlock::create(value_count, bpv)?),
            ));
        }
    }
    Ok(packed_ints)
}

/// `fill(PackedInts.Mutable packedInt, int bitsPerValue, long randomSeed)`.
fn fill(name: &str, packed_int: &mut dyn PackedIntsMutable, bits_per_value: i32, random_seed: i64) {
    let rnd2 = JdkRandom::new(random_seed);
    let max_value = if bits_per_value == 64 {
        i64::MAX
    } else {
        (1i64 << bits_per_value) - 1
    };
    for i in 0..packed_int.size() {
        let value = if bits_per_value == 64 {
            random().next_long()
        } else {
            TestUtil::next_long(&rnd2, 0, max_value)
        };
        packed_int.set(i, value);
        assert_equals_msg(
            Some(&format!(
                "The set/get of the value at index {i} should match for {name}"
            )),
            &value,
            &packed_int.get(i),
        );
    }
}

/// `assertListEquality(List<? extends PackedInts.Reader> packedInts)`.
fn assert_list_equality(packed_ints: &[NamedMutable]) {
    assert_list_equality_msg("", packed_ints)
}

/// `assertListEquality(String message, List<? extends PackedInts.Reader>)`.
fn assert_list_equality_msg(message: &str, packed_ints: &[NamedMutable]) {
    if packed_ints.is_empty() {
        return;
    }
    let (base_name, base) = &packed_ints[0];
    let value_count = base.size();
    for (_, packed_int) in packed_ints {
        assert_equals_msg(
            Some(&format!(
                "{message}. The number of values should be the same "
            )),
            &value_count,
            &packed_int.size(),
        );
    }
    for i in 0..value_count {
        for j in 1..packed_ints.len() {
            assert_equals_msg(
                Some(&format!(
                    "{message}. The value at index {i} should be the same for {base_name} and {}",
                    packed_ints[j].0
                )),
                &base.get(i),
                &packed_ints[j].1.get(i),
            );
        }
    }
}

fn test_secondary_block_change() -> Result<()> {
    let mut mutable = Packed64::new(26, 5)?;
    mutable.set(24, 31);
    assert_equals_msg(
        Some("The value #24 should be correct"),
        &31,
        &mutable.get(24),
    );
    mutable.set(4, 16);
    assert_equals_msg(
        Some("The value #24 should remain unchanged"),
        &31,
        &mutable.get(24),
    );
    Ok(())
}

fn test_fill() -> Result<()> {
    let value_count = 1111;
    let from = random().next_int_bound(value_count + 1);
    let to = from + random().next_int_bound(value_count + 1 - from);
    for bpv in 1..=64 {
        let val = TestUtil::next_long(&*random(), 0, PackedInts::max_value(bpv));
        let mut packed_ints = create_packed_ints(value_count, bpv)?;
        for (name, ints) in packed_ints.iter_mut() {
            let msg = format!("{name} bpv={bpv}, from={from}, to={to}, val={val}");
            let size = ints.size();
            ints.fill(0, size, 1);
            ints.fill(from, to, val);
            for i in 0..ints.size() {
                if i >= from && i < to {
                    assert_equals_msg(Some(&format!("{msg}, i={i}")), &val, &ints.get(i));
                } else {
                    assert_equals_msg(Some(&format!("{msg}, i={i}")), &1, &ints.get(i));
                }
            }
        }
    }
    Ok(())
}

fn test_packed_ints_null() -> Result<()> {
    // must be > 10 for the bulk reads below
    let size = TestUtil::next_int(&*random(), 11, 256);
    let packed_ints = NullReader::for_count(size);
    assert_equals(
        &0,
        &packed_ints.get(TestUtil::next_int(&*random(), 0, size - 1)),
    );
    let mut arr = vec![0i64; (size + 10) as usize];
    arr.fill(1);
    let mut r = packed_ints.get_bulk(0, &mut arr, 0, (size - 1) as usize);
    assert_equals(&(size - 1), &r);
    r -= 1;
    while r >= 0 {
        assert_equals(&0, &arr[r as usize]);
        r -= 1;
    }
    arr.fill(1);
    let r = packed_ints.get_bulk(10, &mut arr, 0, (size + 10) as usize);
    assert_equals(&(size - 10), &r);
    for i in 0..(size - 10) as usize {
        assert_equals(&0, &arr[i]);
    }
    Ok(())
}

fn test_bulk_get() -> Result<()> {
    let value_count = 1111;
    let index = random().next_int_bound(value_count);
    let len = TestUtil::next_int(&*random(), 1, value_count * 2);
    let off = random().next_int_bound(77);

    for bpv in 1..=64 {
        let mask = PackedInts::max_value(bpv);
        let mut packed_ints = create_packed_ints(value_count, bpv)?;

        for (name, ints) in packed_ints.iter_mut() {
            for i in 0..ints.size() {
                ints.set(i, (31i64.wrapping_mul(i as i64) - 1099) & mask);
            }
            let mut arr = vec![0i64; (off + len) as usize];

            let msg =
                format!("{name} valueCount={value_count}, index={index}, len={len}, off={off}");
            let gets = ints.get_bulk(index, &mut arr, off as usize, len as usize);
            Assert::assert_true_msg(&msg, gets > 0);
            Assert::assert_true_msg(&msg, gets <= len);
            Assert::assert_true_msg(&msg, gets <= ints.size() - index);

            for i in 0..arr.len() as i32 {
                let m = format!("{msg}, i={i}");
                if i >= off && i < off + gets {
                    assert_equals_msg(Some(&m), &ints.get(i - off + index), &arr[i as usize]);
                } else {
                    assert_equals_msg(Some(&m), &0, &arr[i as usize]);
                }
            }
        }
    }
    Ok(())
}

fn test_bulk_set() -> Result<()> {
    let value_count = 1111;
    let index = random().next_int_bound(value_count);
    let len = TestUtil::next_int(&*random(), 1, value_count * 2);
    let off = random().next_int_bound(77);
    let mut arr = vec![0i64; (off + len) as usize];

    for bpv in 1..=64 {
        let mask = PackedInts::max_value(bpv);
        let mut packed_ints = create_packed_ints(value_count, bpv)?;
        for i in 0..arr.len() {
            arr[i] = (31i64.wrapping_mul(i as i64) + 19) & mask;
        }

        for (name, ints) in packed_ints.iter_mut() {
            let msg =
                format!("{name} valueCount={value_count}, index={index}, len={len}, off={off}");
            let sets = ints.set_bulk(index, &arr, off as usize, len as usize);
            Assert::assert_true_msg(&msg, sets > 0);
            Assert::assert_true_msg(&msg, sets <= len);

            for i in 0..ints.size() {
                let m = format!("{msg}, i={i}");
                if i >= index && i < index + sets {
                    assert_equals_msg(Some(&m), &arr[(off - index + i) as usize], &ints.get(i));
                } else {
                    assert_equals_msg(Some(&m), &0, &ints.get(i));
                }
            }
        }
    }
    Ok(())
}

fn test_copy() -> Result<()> {
    let value_count = TestUtil::next_int(&*random(), 5, 600);
    let off1 = random().next_int_bound(value_count);
    let off2 = random().next_int_bound(value_count);
    let len = random().next_int_bound((value_count - off1).min(value_count - off2));
    let mem = random().next_int_bound(1024);

    for bpv in 1..=64 {
        let mask = PackedInts::max_value(bpv);
        for (name1, mut r1) in create_packed_ints(value_count, bpv)? {
            for i in 0..r1.size() {
                r1.set(i, (31i64.wrapping_mul(i as i64) - 1023) & mask);
            }
            for (name2, mut r2) in create_packed_ints(value_count, bpv)? {
                let msg = format!(
                    "src={name1}, dest={name2}, srcPos={off1}, destPos={off2}, len={len}, mem={mem}"
                );
                PackedInts::copy(r1.as_ref(), off1, r2.as_mut(), off2, len, mem);
                for i in 0..r2.size() {
                    let m = format!("{msg}, i={i}");
                    if i >= off2 && i < off2 + len {
                        assert_equals_msg(Some(&m), &r1.get(i - off2 + off1), &r2.get(i));
                    } else {
                        assert_equals_msg(Some(&m), &0, &r2.get(i));
                    }
                }
            }
        }
    }
    Ok(())
}

fn test_growable_writer() -> Result<()> {
    let value_count = 113 + random().next_int_bound(1111);
    let mut wrt = GrowableWriter::new(1, value_count, PackedInts::DEFAULT)?;
    wrt.set(4, 2);
    wrt.set(7, 10);
    wrt.set(value_count - 10, 99);
    wrt.set(99, 999);
    wrt.set(value_count - 1, 1 << 10);
    assert_equals(&(1i64 << 10), &wrt.get(value_count - 1));
    wrt.set(99, (1 << 23) - 1);
    assert_equals(&(1i64 << 10), &wrt.get(value_count - 1));
    wrt.set(1, i64::MAX);
    wrt.set(2, -3);
    assert_equals(&64, &wrt.bits_per_value());
    assert_equals(&(1i64 << 10), &wrt.get(value_count - 1));
    assert_equals(&i64::MAX, &wrt.get(1));
    assert_equals(&-3i64, &wrt.get(2));
    assert_equals(&2, &wrt.get(4));
    assert_equals(&((1i64 << 23) - 1), &wrt.get(99));
    assert_equals(&10, &wrt.get(7));
    assert_equals(&99, &wrt.get(value_count - 10));
    assert_equals(&(1i64 << 10), &wrt.get(value_count - 1));
    assert_equals(
        &ram_used_unmodelled("GrowableWriter"),
        &wrt.ram_bytes_used(),
    );
    Ok(())
}

fn test_paged_growable_writer() -> Result<()> {
    let random = random();
    let page_size = 1usize << TestUtil::next_int(&*random, 6, 30);
    // supports 0 values?
    let writer = PagedGrowableWriter::new(
        0,
        page_size,
        TestUtil::next_int(&*random, 1, 64),
        random.next_float(),
    )?;
    assert_equals(&0, &writer.size());

    // compare against AppendingDeltaPackedLongBuffer
    let mut buf = PackedLongValues::delta_packed_builder(
        PackedLongValues::DEFAULT_PAGE_SIZE as usize,
        random.next_float(),
    )?;
    let size = if *TEST_NIGHTLY {
        random.next_int_bound(1000000)
    } else {
        random.next_int_bound(100000)
    };
    let mut max: i64 = 5;
    for _ in 0..size {
        buf.add(TestUtil::next_long(&*random, 0, max))?;
        if LuceneTestCase::rarely_with(&*random) {
            max = PackedInts::max_value(if LuceneTestCase::rarely_with(&*random) {
                TestUtil::next_int(&*random, 0, 63)
            } else {
                TestUtil::next_int(&*random, 0, 31)
            });
        }
    }
    let mut writer = PagedGrowableWriter::new(
        size as i64,
        page_size,
        TestUtil::next_int(&*random, 1, 64),
        random.next_float(),
    )?;
    assert_equals(&(size as i64), &writer.size());
    let values = buf.build()?;
    for i in (0..size as i64).rev() {
        writer.set(i, values.get(i));
    }
    for i in 0..size as i64 {
        assert_equals(&values.get(i), &writer.get(i));
    }

    // test ramBytesUsed
    Assert::assert_equals_double_msg(
        None,
        ram_used_unmodelled("PagedGrowableWriter") as f64,
        writer.ram_bytes_used() as f64,
        8.0,
    );

    // test copy
    let copy = writer.resize(TestUtil::next_long(
        &*random,
        writer.size() / 2,
        writer.size() * 3 / 2,
    ))?;
    for i in 0..copy.size() {
        if i < writer.size() {
            assert_equals(&writer.get(i), &copy.get(i));
        } else {
            assert_equals(&0, &copy.get(i));
        }
    }

    // test grow
    let min_size = TestUtil::next_long(&*random, writer.size() / 2, writer.size() * 3 / 2);
    let writer_size = writer.size();
    let writer_values: Vec<i64> = (0..writer_size).map(|i| writer.get(i)).collect();
    let grow = writer.grow(min_size)?;
    for i in 0..grow.size() {
        if i < writer_size {
            assert_equals(&writer_values[i as usize], &grow.get(i));
        } else {
            assert_equals(&0, &grow.get(i));
        }
    }
    Ok(())
}

fn test_paged_mutable() -> Result<()> {
    let random = random();
    let bits_per_value = TestUtil::next_int(&*random, 1, 64);
    let max = PackedInts::max_value(bits_per_value);
    let page_size = 1usize << TestUtil::next_int(&*random, 6, 30);
    // supports 0 values?
    let writer = PagedMutable::new(0, page_size, bits_per_value, random.next_float() / 2.0)?;
    assert_equals(&0, &writer.size());

    // compare against AppendingDeltaPackedLongBuffer
    let mut buf = PackedLongValues::delta_packed_builder(
        PackedLongValues::DEFAULT_PAGE_SIZE as usize,
        random.next_float(),
    )?;
    let size = if *TEST_NIGHTLY {
        random.next_int_bound(1000000)
    } else {
        random.next_int_bound(100000)
    };
    for _ in 0..size {
        buf.add(if bits_per_value == 64 {
            random.next_long()
        } else {
            TestUtil::next_long(&*random, 0, max)
        })?;
    }
    let mut writer =
        PagedMutable::new(size as i64, page_size, bits_per_value, random.next_float())?;
    assert_equals(&(size as i64), &writer.size());
    let values = buf.build()?;
    for i in (0..size as i64).rev() {
        writer.set(i, values.get(i));
    }
    for i in 0..size as i64 {
        assert_equals(&values.get(i), &writer.get(i));
    }

    // test ramBytesUsed:
    // RamUsageTester.ramUsed(writer) - RamUsageTester.ramUsed(writer.format)
    assert_equals(
        &ram_used_unmodelled("PagedMutable"),
        &writer.ram_bytes_used(),
    );

    // test copy
    let copy = writer.resize(TestUtil::next_long(
        &*random,
        writer.size() / 2,
        writer.size() * 3 / 2,
    ))?;
    for i in 0..copy.size() {
        if i < writer.size() {
            assert_equals(&writer.get(i), &copy.get(i));
        } else {
            assert_equals(&0, &copy.get(i));
        }
    }

    // test grow
    let min_size = TestUtil::next_long(&*random, writer.size() / 2, writer.size() * 3 / 2);
    let writer_size = writer.size();
    let writer_values: Vec<i64> = (0..writer_size).map(|i| writer.get(i)).collect();
    let grow = writer.grow(min_size)?;
    for i in 0..grow.size() {
        if i < writer_size {
            assert_equals(&writer_values[i as usize], &grow.get(i));
        } else {
            assert_equals(&0, &grow.get(i));
        }
    }
    Ok(())
}

/// The big-endian bytes of `longs` (`ByteBuffer.wrap(bytes).asLongBuffer().put(longs)`).
fn longs_to_be_bytes(longs: &[i64]) -> Vec<u8> {
    longs.iter().flat_map(|l| l.to_be_bytes()).collect()
}

/// The longs of big-endian `bytes` (`ByteBuffer.wrap(bytes).asLongBuffer()`).
fn be_bytes_to_longs(bytes: &[u8]) -> Vec<i64> {
    bytes
        .chunks_exact(8)
        .map(|c| i64::from_be_bytes(c.try_into().expect("8 bytes")))
        .collect()
}

fn test_encode_decode() -> Result<()> {
    for format in FORMATS {
        for bpv in 1..=64 {
            if !format.is_supported(bpv) {
                continue;
            }
            let msg = format!("{format:?} {bpv}");

            let encoder = get_encoder(format, PackedInts::VERSION_CURRENT, bpv)?;
            let decoder = get_decoder(format, PackedInts::VERSION_CURRENT, bpv)?;
            let long_block_count = encoder.long_block_count();
            let long_value_count = encoder.long_value_count();
            let byte_block_count = encoder.byte_block_count();
            let byte_value_count = encoder.byte_value_count();
            assert_equals(&long_block_count, &decoder.long_block_count());
            assert_equals(&long_value_count, &decoder.long_value_count());
            assert_equals(&byte_block_count, &decoder.byte_block_count());
            assert_equals(&byte_value_count, &decoder.byte_value_count());

            let long_iterations = random().next_int_bound(100) as usize;
            let byte_iterations = long_iterations * long_value_count / byte_value_count;
            assert_equals(
                &(long_iterations * long_value_count),
                &(byte_iterations * byte_value_count),
            );
            let blocks_offset = random().next_int_bound(100) as usize;
            let values_offset = random().next_int_bound(100) as usize;
            let blocks_offset2 = random().next_int_bound(100) as usize;
            let blocks_len = long_iterations * long_block_count;

            // 1. generate random inputs
            let mut blocks = vec![0i64; blocks_offset + blocks_len];
            for i in 0..blocks.len() {
                blocks[i] = random().next_long();
                if format == Format::PackedSingleBlock && 64 % bpv != 0 {
                    // clear highest bits for packed
                    let to_clear = 64 % bpv;
                    blocks[i] = ((blocks[i] << to_clear) as u64 >> to_clear) as i64;
                }
            }

            // 2. decode
            let mut values = vec![0i64; values_offset + long_iterations * long_value_count];
            decoder.decode_long_blocks_to_longs(
                &blocks,
                blocks_offset,
                &mut values,
                values_offset,
                long_iterations,
            )?;
            for &value in &values {
                Assert::assert_true(value <= PackedInts::max_value(bpv));
            }
            // test decoding to int[]
            let int_values: Option<Vec<i32>> = if bpv <= 32 {
                let mut int_values = vec![0i32; values.len()];
                decoder.decode_long_blocks_to_ints(
                    &blocks,
                    blocks_offset,
                    &mut int_values,
                    values_offset,
                    long_iterations,
                )?;
                Assert::assert_true(equals(&int_values, &values));
                Some(int_values)
            } else {
                None
            };

            // 3. re-encode
            let mut blocks2 = vec![0i64; blocks_offset2 + blocks_len];
            encoder.encode_longs_to_long_blocks(
                &values,
                values_offset,
                &mut blocks2,
                blocks_offset2,
                long_iterations,
            )?;
            assert_array_equals_msg(
                Some(&msg),
                &ArrayUtil::copy_of_sub_array_long(&blocks, blocks_offset, blocks.len()),
                &ArrayUtil::copy_of_sub_array_long(&blocks2, blocks_offset2, blocks2.len()),
            );
            // test encoding from int[]
            if let Some(int_values) = &int_values {
                let mut blocks3 = vec![0i64; blocks2.len()];
                encoder.encode_ints_to_long_blocks(
                    int_values,
                    values_offset,
                    &mut blocks3,
                    blocks_offset2,
                    long_iterations,
                )?;
                assert_array_equals_msg(Some(&msg), &blocks2, &blocks3);
            }

            // 4. byte[] decoding
            let byte_blocks = longs_to_be_bytes(&blocks);
            let mut values2 = vec![0i64; values_offset + long_iterations * long_value_count];
            decoder.decode_byte_blocks_to_longs(
                &byte_blocks,
                blocks_offset * 8,
                &mut values2,
                values_offset,
                byte_iterations,
            )?;
            for &value in &values2 {
                Assert::assert_true_msg(&msg, value <= PackedInts::max_value(bpv));
            }
            assert_array_equals_msg(Some(&msg), &values, &values2);
            // test decoding to int[]
            if bpv <= 32 {
                let mut int_values2 = vec![0i32; values2.len()];
                decoder.decode_byte_blocks_to_ints(
                    &byte_blocks,
                    blocks_offset * 8,
                    &mut int_values2,
                    values_offset,
                    byte_iterations,
                )?;
                Assert::assert_true_msg(&msg, equals(&int_values2, &values2));
            }

            // 5. byte[] encoding
            let mut blocks3 = vec![0u8; 8 * (blocks_offset2 + blocks_len)];
            encoder.encode_longs_to_byte_blocks(
                &values,
                values_offset,
                &mut blocks3,
                8 * blocks_offset2,
                byte_iterations,
            )?;
            // assertEquals(LongBuffer.wrap(blocks2), ByteBuffer.wrap(blocks3).asLongBuffer())
            assert_equals_msg(Some(&msg), &blocks2, &be_bytes_to_longs(&blocks3));
            // test encoding from int[]
            if let Some(int_values) = &int_values {
                let mut blocks4 = vec![0u8; blocks3.len()];
                encoder.encode_ints_to_byte_blocks(
                    int_values,
                    values_offset,
                    &mut blocks4,
                    8 * blocks_offset2,
                    byte_iterations,
                )?;
                assert_array_equals_msg(Some(&msg), &blocks3, &blocks4);
            }
        }
    }
    Ok(())
}

/// `equals(int[] ints, long[] longs)`.
fn equals(ints: &[i32], longs: &[i64]) -> bool {
    if ints.len() != longs.len() {
        return false;
    }
    for i in 0..ints.len() {
        if (ints[i] as i64 & 0xFFFFFFFF) != longs[i] {
            return false;
        }
    }
    true
}

/// `enum DataType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DataType {
    Packed,
    DeltaPacked,
    Monotonic,
}

/// `DataType.values()`.
const DATA_TYPES: [DataType; 3] = [DataType::Packed, DataType::DeltaPacked, DataType::Monotonic];

fn test_packed_long_values_on_zeros() -> Result<()> {
    // Make sure that when all values are the same, they use 0 bits per value
    let page_size = 1usize << TestUtil::next_int(&*random(), 6, 20);
    let acceptable_overhead_ratio = random().next_float();

    let ram_of_packed = |values: &[i64]| -> Result<i64> {
        let mut b = PackedLongValues::packed_builder(page_size, acceptable_overhead_ratio)?;
        for &v in values {
            b.add(v)?;
        }
        Ok(b.build()?.ram_bytes_used())
    };
    assert_equals(&ram_of_packed(&[0])?, &ram_of_packed(&[0, 0])?);

    let l = random().next_long();
    let ram_of_delta = |values: &[i64]| -> Result<i64> {
        let mut b = PackedLongValues::delta_packed_builder(page_size, acceptable_overhead_ratio)?;
        for &v in values {
            b.add(v)?;
        }
        Ok(b.build()?.ram_bytes_used())
    };
    assert_equals(&ram_of_delta(&[l])?, &ram_of_delta(&[l, l])?);

    let avg = random().next_int_bound(100) as i64;
    let ram_of_monotonic = |values: &[i64]| -> Result<i64> {
        let mut b = PackedLongValues::monotonic_builder(page_size, acceptable_overhead_ratio)?;
        for &v in values {
            b.add(v)?;
        }
        Ok(b.build()?.ram_bytes_used())
    };
    assert_equals(
        &ram_of_monotonic(&[l, l.wrapping_add(avg)])?,
        &ram_of_monotonic(&[l, l.wrapping_add(avg), l.wrapping_add(2 * avg)])?,
    );
    Ok(())
}

/// The body of `testPackedLongValues` after the builder is chosen: `buf` is
/// filled with `arr`, built with `build`, and checked.
macro_rules! check_packed_long_values {
    ($buf:expr, $arr:expr) => {{
        let mut buf = $buf;
        let arr: &[i64] = $arr;
        for i in 0..arr.len() {
            buf.add(arr[i])?;
            if LuceneTestCase::rarely() && !*TEST_NIGHTLY {
                let expected_bytes_used = ram_used_unmodelled("PackedLongValues.Builder");
                let computed_bytes_used = buf.ram_bytes_used();
                assert_equals(&expected_bytes_used, &computed_bytes_used);
            }
        }
        assert_equals(&(arr.len() as i64), &buf.size());
        let values = buf.build()?;
        // expectThrows(IllegalStateException.class, () -> buf.add(...)):
        // not expressible, `build` consumes the builder.
        assert_equals(&(arr.len() as i64), &values.size());
        for i in 0..arr.len() {
            assert_equals(&arr[i], &values.get(i as i64));
        }
        let mut it = values.iterator();
        for i in 0..arr.len() {
            if random().next_boolean() {
                Assert::assert_true(it.has_next());
            }
            assert_equals(&arr[i], &it.next_value());
        }
        Assert::assert_false(it.has_next());
        let expected_bytes_used = ram_used_unmodelled("PackedLongValues");
        let computed_bytes_used = values.ram_bytes_used();
        assert_equals(&expected_bytes_used, &computed_bytes_used);
    }};
}

fn test_packed_long_values() -> Result<()> {
    let mut arr = vec![
        0i64;
        RandomNumbers::random_int_between(
            &*random(),
            1,
            if *TEST_NIGHTLY { 1000000 } else { 10000 }
        ) as usize
    ];
    let ratio_options = [PackedInts::DEFAULT, PackedInts::COMPACT, PackedInts::FAST];
    for bpv in [
        0,
        1,
        63,
        64,
        RandomNumbers::random_int_between(&*random(), 2, 62),
    ] {
        for data_type in DATA_TYPES {
            let page_size = 1usize << TestUtil::next_int(&*random(), 6, 20);
            let acceptable_overhead_ratio = ratio_options
                [TestUtil::next_int(&*random(), 0, ratio_options.len() as i32 - 1) as usize];
            let inc: i64 = match data_type {
                DataType::Packed | DataType::DeltaPacked => 0,
                DataType::Monotonic => TestUtil::next_int(&*random(), -1000, 1000) as i64,
            };

            if bpv == 0 {
                arr[0] = random().next_long();
                for i in 1..arr.len() {
                    arr[i] = arr[i - 1].wrapping_add(inc);
                }
            } else if bpv == 64 {
                for i in 0..arr.len() {
                    arr[i] = random().next_long();
                }
            } else {
                let min_value = TestUtil::next_long(
                    &*random(),
                    i64::MIN,
                    i64::MAX - PackedInts::max_value(bpv),
                );
                for i in 0..arr.len() {
                    // _TestUtil.nextLong is too slow
                    arr[i] = min_value
                        .wrapping_add(inc.wrapping_mul(i as i64))
                        .wrapping_add(random().next_long())
                        & PackedInts::max_value(bpv);
                }
            }

            match data_type {
                DataType::Packed => check_packed_long_values!(
                    PackedLongValues::packed_builder(page_size, acceptable_overhead_ratio)?,
                    &arr
                ),
                DataType::DeltaPacked => check_packed_long_values!(
                    PackedLongValues::delta_packed_builder(page_size, acceptable_overhead_ratio)?,
                    &arr
                ),
                DataType::Monotonic => check_packed_long_values!(
                    PackedLongValues::monotonic_builder(page_size, acceptable_overhead_ratio)?,
                    &arr
                ),
            }
        }
    }
    Ok(())
}

/// The body of `testPackedLongValuesAddRepeat` after the builder is chosen.
macro_rules! check_add_repeat {
    ($buf:expr, $page_size:expr) => {{
        let mut buf = $buf;
        let page_size: usize = $page_size;
        let mut expected: Vec<i64> = Vec::new();
        let num_ops = LuceneTestCase::at_least(20);
        for _ in 0..num_ops {
            match random().next_int_bound(3) {
                0 => {
                    let single = random().next_long();
                    buf.add(single)?;
                    expected.push(single);
                }
                1 => {
                    buf.add_repeated(random().next_long(), 0)?;
                }
                _ => {
                    let value = random().next_long();
                    let count = TestUtil::next_int(&*random(), 1, page_size as i32 * 3);
                    buf.add_repeated(value, count)?;
                    for _ in 0..count {
                        expected.push(value);
                    }
                }
            }
            assert_equals(&(expected.len() as i64), &buf.size());
        }
        let values = buf.build()?;
        assert_equals(&(expected.len() as i64), &values.size());
        for i in 0..expected.len() {
            assert_equals(&expected[i], &values.get(i as i64));
        }
        let mut it = values.iterator();
        for i in 0..expected.len() {
            Assert::assert_true(it.has_next());
            assert_equals(&expected[i], &it.next_value());
        }
        Assert::assert_false(it.has_next());
        // expectThrows(IllegalStateException.class, () -> buf.add(..., 5)):
        // not expressible, `build` consumes the builder.
    }};
}

fn test_packed_long_values_add_repeat() -> Result<()> {
    for data_type in DATA_TYPES {
        let page_size = 1usize << TestUtil::next_int(&*random(), 6, 12);
        let acceptable_overhead_ratio = random().next_float();
        match data_type {
            DataType::Packed => check_add_repeat!(
                PackedLongValues::packed_builder(page_size, acceptable_overhead_ratio)?,
                page_size
            ),
            DataType::DeltaPacked => check_add_repeat!(
                PackedLongValues::delta_packed_builder(page_size, acceptable_overhead_ratio)?,
                page_size
            ),
            DataType::Monotonic => check_add_repeat!(
                PackedLongValues::monotonic_builder(page_size, acceptable_overhead_ratio)?,
                page_size
            ),
        }
    }
    Ok(())
}

fn test_packed_input_output() -> Result<()> {
    let n = random().next_int_bound(8192) as usize;
    let mut longs = vec![0i64; n];
    let mut bits_per_values = vec![0i32; n];
    let mut skip = vec![false; n];
    for i in 0..n {
        let bpv = RandomNumbers::random_int_between(&*random(), 1, 64);
        bits_per_values[i] = if random().next_boolean() {
            bpv
        } else {
            TestUtil::next_int(&*random(), bpv, 64)
        };
        longs[i] = if bpv == 64 {
            random().next_long()
        } else {
            TestUtil::next_long(&*random(), 0, PackedInts::max_value(bpv))
        };
        skip[i] = LuceneTestCase::rarely();
    }

    let mut d = LuceneTestCase::new_directory();
    let dir = d.as_directory();
    let mut out = dir.create_output("out.bin", &*DEFAULT_IO_CONTEXT)?;
    let mut total_bits: i64 = 0;
    {
        let mut pout = PackedDataOutput::new(out.as_mut());
        for i in 0..n {
            pout.write_long(longs[i], bits_per_values[i])?;
            total_bits += bits_per_values[i] as i64;
            if skip[i] {
                pout.flush()?;
                total_bits = 8 * (total_bits as f64 / 8.0).ceil() as i64;
            }
        }
        pout.flush()?;
    }
    assert_equals(
        &((total_bits as f64 / 8.0).ceil() as i64),
        &out.file_pointer(),
    );
    out.close()?;
    let mut r#in = dir.open_input("out.bin", &*READONCE_IO_CONTEXT)?;
    {
        let mut pin = PackedDataInput::new(r#in.as_mut());
        for i in 0..n {
            assert_equals_msg(
                Some(&format!("{i}")),
                &longs[i],
                &pin.read_long(bits_per_values[i])?,
            );
            if skip[i] {
                pin.skip_to_next_byte();
            }
        }
    }
    assert_equals(
        &((total_bits as f64 / 8.0).ceil() as i64),
        &r#in.file_pointer(),
    );
    r#in.close()?;
    drop(dir);
    d.close()
}

/// The reading half of `testBlockPackedReaderWriter` over `in` (an
/// `IndexInput` or a `ByteArrayDataInput`): `position` is
/// `getFilePointer()`/`getPosition()` and `rewind` is `seek(0)`/
/// `setPosition(0)`.
fn check_block_packed_reads<I: DataInput + ?Sized>(
    input: &mut I,
    position: fn(&mut I) -> i64,
    rewind: fn(&mut I) -> Result<()>,
    random: &dyn Random,
    values: &[i64],
    block_size: usize,
    fp: i64,
) -> Result<()> {
    let value_count = values.len();
    {
        let mut it = BlockPackedReaderIterator::new(
            &mut *input,
            PackedInts::VERSION_CURRENT,
            block_size,
            value_count as i64,
        )?;
        let mut i = 0usize;
        while i < value_count {
            if random.next_boolean() {
                assert_equals_msg(Some(&format!("{i}")), &values[i], &it.next()?);
                i += 1;
            } else {
                let next_values = it.next_batch(TestUtil::next_int(random, 1, 1024) as usize)?;
                for j in 0..next_values.len() {
                    assert_equals_msg(Some(&format!("{}", i + j)), &values[i + j], &next_values[j]);
                }
                i += next_values.len();
            }
            assert_equals(&(i as i64), &it.ord());
        }
        assert_equals(&fp, &position(it.input()));
        expect_throws("java.io.IOException", || it.next());
    }
    rewind(input)?;

    {
        let mut it2 = BlockPackedReaderIterator::new(
            &mut *input,
            PackedInts::VERSION_CURRENT,
            block_size,
            value_count as i64,
        )?;
        let mut i = 0usize;
        loop {
            let skip = TestUtil::next_int(random, 0, (value_count - i) as i32) as usize;
            it2.skip(skip as i64)?;
            i += skip;
            assert_equals(&(i as i64), &it2.ord());
            if i == value_count {
                break;
            } else {
                assert_equals(&values[i], &it2.next()?);
                i += 1;
            }
        }
        assert_equals(&fp, &position(it2.input()));
        expect_throws("java.io.IOException", || it2.skip(1));
    }
    Ok(())
}

fn test_block_packed_reader_writer() -> Result<()> {
    let random = random();
    let iters = LuceneTestCase::at_least(2);
    for _ in 0..iters {
        let block_size = 1usize << TestUtil::next_int(&*random, 6, 18);
        let value_count = if *TEST_NIGHTLY {
            random.next_int_bound(1 << 18)
        } else {
            random.next_int_bound(1 << 15)
        } as usize;
        let mut values = vec![0i64; value_count];
        let mut min_value: i64 = 0;
        let mut bpv = 0;
        for i in 0..value_count {
            if i % block_size == 0 {
                min_value = if LuceneTestCase::rarely_with(&*random) {
                    random.next_int_bound(256) as i64
                } else if LuceneTestCase::rarely_with(&*random) {
                    -5
                } else {
                    random.next_long()
                };
                bpv = random.next_int_bound(65);
            }
            values[i] = if bpv == 0 {
                min_value
            } else if bpv == 64 {
                random.next_long()
            } else {
                min_value.wrapping_add(TestUtil::next_long(&*random, 0, (1i64 << bpv) - 1))
            };
        }

        let mut d = LuceneTestCase::new_directory();
        let dir = d.as_directory();
        let mut out = dir.create_output("out.bin", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut writer = BlockPackedWriter::new(out.as_mut(), block_size)?;
            for i in 0..value_count {
                assert_equals(&(i as i64), &writer.ord());
                writer.add(values[i])?;
            }
            assert_equals(&(value_count as i64), &writer.ord());
            writer.finish()?;
            assert_equals(&(value_count as i64), &writer.ord());
        }
        let fp = out.file_pointer();
        out.close()?;

        let mut in1 = dir.open_input("out.bin", &*DEFAULT_IO_CONTEXT)?;
        let mut buf = vec![0u8; fp as usize];
        in1.read_bytes(&mut buf, 0, fp as usize)?;
        in1.seek(0)?;
        let mut in2 = ByteArrayDataInput::new(&buf);
        if random.next_boolean() {
            check_block_packed_reads(
                in1.as_mut(),
                |i| i.file_pointer(),
                |i| i.seek(0),
                &*random,
                &values,
                block_size,
                fp,
            )?;
        } else {
            check_block_packed_reads(
                &mut in2,
                |i| i.position() as i64,
                |i| {
                    i.set_position(0);
                    Ok(())
                },
                &*random,
                &values,
                block_size,
                fp,
            )?;
        }
        in1.close()?;
        drop(dir);
        d.close()?;
    }
    Ok(())
}

fn test_monotonic_block_packed_reader_writer() -> Result<()> {
    let iters = LuceneTestCase::at_least(2);
    for _ in 0..iters {
        let block_size = 1usize << TestUtil::next_int(&*random(), 6, 18);
        let value_count = random().next_int_bound(1 << 18) as usize;
        let mut values = vec![0i64; value_count];
        if value_count > 0 {
            values[0] = if random().next_boolean() {
                random().next_int_bound(10)
            } else {
                random().next_int_bound(i32::MAX)
            } as i64;
            let mut max_delta = random().next_int_bound(64);
            for i in 1..value_count {
                if random().next_double() < 0.1 {
                    max_delta = random().next_int_bound(64);
                }
                values[i] =
                    0.max(values[i - 1] + TestUtil::next_int(&*random(), -16, max_delta) as i64);
            }
        }

        let mut d = LuceneTestCase::new_directory();
        let dir = d.as_directory();
        let mut out = dir.create_output("out.bin", &*DEFAULT_IO_CONTEXT)?;
        {
            let mut writer = MonotonicBlockPackedWriter::new(out.as_mut(), block_size)?;
            for i in 0..value_count {
                assert_equals(&(i as i64), &writer.ord());
                writer.add(values[i])?;
            }
            assert_equals(&(value_count as i64), &writer.ord());
            writer.finish()?;
            assert_equals(&(value_count as i64), &writer.ord());
        }
        let fp = out.file_pointer();
        out.close()?;

        let mut r#in = dir.open_input("out.bin", &*DEFAULT_IO_CONTEXT)?;
        let reader = MonotonicBlockPackedReader::of(
            r#in.as_mut(),
            PackedInts::VERSION_CURRENT,
            block_size,
            value_count as i64,
        )?;
        assert_equals(&fp, &r#in.file_pointer());
        for i in 0..value_count {
            assert_equals_msg(Some(&format!("i={i}")), &values[i], &reader.get(i as i64));
        }
        r#in.close()?;
        drop(dir);
        d.close()?;
    }
    Ok(())
}

fn test_block_reader_overflow() -> Result<()> {
    let value_count = TestUtil::next_long(&*random(), 1 + i32::MAX as i64, i32::MAX as i64 * 2);
    let block_size = 1i64 << TestUtil::next_int(&*random(), 20, 22);
    let mut d = LuceneTestCase::new_directory();
    let dir = d.as_directory();
    let mut out = dir.create_output("out.bin", &*DEFAULT_IO_CONTEXT)?;
    let value = random().next_int() as i64 & 0xFFFFFFFF;
    let value_offset = TestUtil::next_long(&*random(), 0, value_count - 1);
    {
        let mut writer = BlockPackedWriter::new(out.as_mut(), block_size as usize)?;
        let mut i: i64 = 0;
        while i < value_count {
            assert_equals(&i, &writer.ord());
            if (i & (block_size - 1)) == 0
                && (i + block_size < value_offset
                    || i > value_offset && i + block_size < value_count)
            {
                writer.add_block_of_zeros()?;
                i += block_size;
            } else if i == value_offset {
                writer.add(value)?;
                i += 1;
            } else {
                writer.add(0)?;
                i += 1;
            }
        }
        writer.finish()?;
    }
    out.close()?;
    let mut r#in = dir.open_input("out.bin", &*DEFAULT_IO_CONTEXT)?;
    {
        let mut it = BlockPackedReaderIterator::new(
            r#in.as_mut(),
            PackedInts::VERSION_CURRENT,
            block_size as usize,
            value_count,
        )?;
        it.skip(value_offset)?;
        assert_equals(&value, &it.next()?);
    }
    r#in.close()?;
    drop(dir);
    d.close()
}
