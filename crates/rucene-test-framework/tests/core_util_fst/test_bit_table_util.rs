//! Port of `lucene/core/src/test/org/apache/lucene/util/fst/TestBitTableUtil.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::store::DataInput;
use rucene::util::fst::{BitTableUtil, BytesReader};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_msg, class, random, test};

/// `TestBitTableUtil`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testNextBitSet", test_next_bit_set),
        test("testPreviousBitSet", test_previous_bit_set),
    ];
    class("org.apache.lucene.util.fst.TestBitTableUtil", methods)
}

fn test_next_bit_set() -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1000);
    for i in 0..num_iterations {
        let bits = build_random_bits();
        let num_bytes = bits.len() as i32 - 1;
        let num_bits = num_bytes * 8;
        for bit_index in -1..num_bits {
            let next_index = BitTableUtil::next_bit_set(bit_index, num_bytes, &mut reader(&bits))?;
            if next_index == -1 {
                assert_equals_msg(
                    Some(&format!(
                        "No next bit set, so expected no bit count diff (i={i} bitIndex={bit_index})"
                    )),
                    &BitTableUtil::count_bits_up_to(bit_index + 1, &mut reader(&bits))?,
                    &BitTableUtil::count_bits(num_bytes, &mut reader(&bits))?,
                );
            } else {
                Assert::assert_true_msg(
                    &format!(
                        "Expected next bit set at nextIndex={next_index} (i={i} bitIndex={bit_index})"
                    ),
                    BitTableUtil::is_bit_set(next_index, &mut reader(&bits))?,
                );
                assert_equals_msg(
                    Some(&format!(
                        "Next bit set at nextIndex={next_index} so expected bit count diff of 1 (i={i} bitIndex={bit_index})"
                    )),
                    &(BitTableUtil::count_bits_up_to(bit_index + 1, &mut reader(&bits))? + 1),
                    &BitTableUtil::count_bits_up_to(next_index + 1, &mut reader(&bits))?,
                );
            }
        }
    }
    Ok(())
}

fn test_previous_bit_set() -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1000);
    for i in 0..num_iterations {
        let bits = build_random_bits();
        let num_bytes = bits.len() as i32 - 1;
        let num_bits = num_bytes * 8;
        for bit_index in 0..=num_bits {
            let previous_index = BitTableUtil::previous_bit_set(bit_index, &mut reader(&bits))?;
            if previous_index == -1 {
                assert_equals_msg(
                    Some(&format!(
                        "No previous bit set, so expected bit count 0 (i={i} bitIndex={bit_index})"
                    )),
                    &0,
                    &BitTableUtil::count_bits_up_to(bit_index, &mut reader(&bits))?,
                );
            } else {
                Assert::assert_true_msg(
                    &format!(
                        "Expected previous bit set at previousIndex={previous_index} (i={i} bitIndex={bit_index})"
                    ),
                    BitTableUtil::is_bit_set(previous_index, &mut reader(&bits))?,
                );
                let bit_count = BitTableUtil::count_bits_up_to(
                    (bit_index + 1).min(num_bits),
                    &mut reader(&bits),
                )?;
                let expected_previous_bit_count = if bit_index < num_bits
                    && BitTableUtil::is_bit_set(bit_index, &mut reader(&bits))?
                {
                    bit_count - 1
                } else {
                    bit_count
                };
                assert_equals_msg(
                    Some(&format!(
                        "Previous bit set at previousIndex={previous_index} with current bitCount={bit_count} so expected previousBitCount={expected_previous_bit_count} (i={i} bitIndex={bit_index})"
                    )),
                    &expected_previous_bit_count,
                    &BitTableUtil::count_bits_up_to(previous_index + 1, &mut reader(&bits))?,
                );
            }
        }
    }
    Ok(())
}

/// `buildRandomBits()`.
fn build_random_bits() -> Vec<u8> {
    let mut bits = vec![0u8; random().next_int_bound(24) as usize + 2];
    for b in bits.iter_mut() {
        *b = if random().next_int_bound(4) == 0 {
            0
        } else {
            random().next_int() as u8
        };
    }
    bits
}

/// `reader(bits)`.
fn reader(bits: &[u8]) -> ByteArrayBytesReader<'_> {
    ByteArrayBytesReader { bits, position: 0 }
}

/// `ByteArrayBytesReader extends FST.BytesReader`.
struct ByteArrayBytesReader<'a> {
    bits: &'a [u8],
    position: i32,
}

impl DataInput for ByteArrayBytesReader<'_> {
    fn read_byte(&mut self) -> Result<u8> {
        let b = self.bits[self.position as usize];
        self.position += 1;
        Ok(b)
    }

    fn to_java_string(&self) -> String {
        "ByteArrayBytesReader".to_string()
    }

    fn read_bytes(&mut self, _b: Option<&mut [u8]>, _offset: usize, _len: usize) -> Result<()> {
        let _b = rucene::store::require_array(_b)?;
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.position += num_bytes as i32;
        Ok(())
    }
}

impl BytesReader for ByteArrayBytesReader<'_> {
    fn position(&self) -> i64 {
        self.position as i64
    }

    fn set_position(&mut self, pos: i64) {
        self.position = pos as i32;
    }
}
