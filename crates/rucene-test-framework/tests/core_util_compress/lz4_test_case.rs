//! Port of `lucene/core/src/test/org/apache/lucene/util/compress/LZ4TestCase.java`:
//! the abstract base of `TestFastLZ4` and `TestHighLZ4`, whose only member
//! is `newHashTable()`.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::store::{ByteArrayDataInput, ByteBuffersDataOutput, DataOutput};
use rucene::util::compress::{
    Lz4, Lz4HashTable, LZ4_LAST_LITERALS, LZ4_MAX_DISTANCE, LZ4_MIN_MATCH,
};
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_array_equals, assert_equals, class, random};

/// `protected abstract LZ4.HashTable newHashTable()`.
pub type NewHashTable = fn() -> Box<dyn Lz4HashTable>;

/// `AssertingHashTable`: delegates to `in`, asserting it is reset when
/// `reset` returns and when `initDictionary` starts.
pub struct AssertingHashTable {
    r#in: Box<dyn Lz4HashTable>,
}

impl AssertingHashTable {
    /// `AssertingHashTable(in)`.
    pub fn new(r#in: Box<dyn Lz4HashTable>) -> Self {
        AssertingHashTable { r#in }
    }
}

impl Lz4HashTable for AssertingHashTable {
    fn reset(&mut self, bytes: &[u8], off: usize, len: usize) -> Result<()> {
        self.r#in.reset(bytes, off, len)?;
        Assert::assert_true(self.r#in.assert_reset());
        Ok(())
    }

    fn init_dictionary(&mut self, bytes: &[u8], dict_len: usize) {
        Assert::assert_true(self.r#in.assert_reset());
        self.r#in.init_dictionary(bytes, dict_len);
    }

    fn get(&mut self, bytes: &[u8], off: usize) -> Option<usize> {
        self.r#in.get(bytes, off)
    }

    fn previous(&mut self, bytes: &[u8], off: usize) -> Option<usize> {
        self.r#in.previous(bytes, off)
    }

    /// `assertReset()`: unsupported.
    fn assert_reset(&self) -> bool {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.UnsupportedOperationException",
            String::new(),
        )
        .throw()
    }
}

/// The tests of `LZ4TestCase`, declared on the subclass `name` whose
/// `newHashTable()` is `new_hash_table`.
pub fn class_def(name: &str, new_hash_table: NewHashTable) -> Arc<Class> {
    let t = |method: &str, body: fn(NewHashTable) -> Result<()>| {
        Method::new(method)
            .throws(&["java.io.IOException"])
            .body_any(move || {
                if let Err(e) = body(new_hash_table) {
                    rucene_test_framework::util::java_exceptions::rethrow_java(e)
                }
            })
    };
    let methods: Vec<Method> = vec![
        t("testEmpty", test_empty),
        t("testShortLiteralsAndMatchs", test_short_literals_and_matchs),
        t("testLongMatchs", test_long_matchs),
        t("testLongLiterals", test_long_literals),
        t(
            "testMatchRightBeforeLastLiterals",
            test_match_right_before_last_literals,
        ),
        t("testIncompressibleRandom", test_incompressible_random),
        t("testCompressibleRandom", test_compressible_random),
        t("testLUCENE5201", test_lucene5201),
        t("testUseDictionary", test_use_dictionary),
    ];
    class(name, methods)
}

/// `ArrayUtil.copyOfSubArray(array, from, to)`.
fn sub(array: &[u8], from: usize, to: usize) -> Vec<u8> {
    array[from..to].to_vec()
}

/// `doTest(data, hashTable)`.
fn do_test(data: &[u8], hash_table: &mut dyn Lz4HashTable) -> Result<()> {
    let offset = if data.len() >= (1 << 16) || random().next_boolean() {
        random().next_int_bound(10) as usize
    } else {
        (1 << 16) - data.len() / 2 // this triggers special reset logic for high compression
    };
    let mut copy = vec![0u8; data.len() + offset + random().next_int_bound(10) as usize];
    copy[offset..offset + data.len()].copy_from_slice(data);
    do_test_range(&copy, offset, data.len(), hash_table)
}

/// `doTest(data, offset, length, hashTable)`.
fn do_test_range(
    data: &[u8],
    offset: usize,
    length: usize,
    hash_table: &mut dyn Lz4HashTable,
) -> Result<()> {
    let mut out = ByteBuffersDataOutput::new();
    Lz4::compress(data, offset, length, &mut out, hash_table)?;
    let compressed = out.to_array_copy()?;

    let mut off = 0usize;
    let mut decompressed_off = 0usize;
    loop {
        let token = compressed[off] as usize;
        off += 1;
        let mut literal_len = token >> 4;
        if literal_len == 0x0F {
            while compressed[off] == 0xFF {
                literal_len += 0xFF;
                off += 1;
            }
            literal_len += compressed[off] as usize;
            off += 1;
        }
        // skip literals
        off += literal_len;
        decompressed_off += literal_len;

        // check that the stream ends with literals and that there are at least
        // 5 of them
        if off == compressed.len() {
            assert_equals(&length, &decompressed_off);
            Assert::assert_true_msg(
                &format!("lastLiterals={literal_len}, bytes={length}"),
                literal_len >= LZ4_LAST_LITERALS || literal_len == length,
            );
            break;
        }

        let match_dec = (compressed[off] as usize) | ((compressed[off + 1] as usize) << 8);
        off += 2;
        // check that match dec is not 0
        Assert::assert_true_msg(
            &format!("{match_dec} {decompressed_off}"),
            match_dec > 0 && match_dec <= decompressed_off,
        );

        let mut match_len = token & 0x0F;
        if match_len == 0x0F {
            while compressed[off] == 0xFF {
                match_len += 0xFF;
                off += 1;
            }
            match_len += compressed[off] as usize;
            off += 1;
        }
        match_len += LZ4_MIN_MATCH;

        // if the match ends prematurely, the next sequence should not have
        // literals or this means we are wasting space
        if decompressed_off + match_len < length.wrapping_sub(LZ4_LAST_LITERALS)
            && length >= LZ4_LAST_LITERALS
        {
            let more_common_bytes = data[offset + decompressed_off + match_len]
                == data[offset + decompressed_off - match_dec + match_len];
            let next_sequence_has_literals = (compressed[off] >> 4) != 0;
            Assert::assert_true(!more_common_bytes || !next_sequence_has_literals);
        }

        decompressed_off += match_len;
    }
    assert_equals(&length, &decompressed_off);

    // Compress once again with the same hash table to test reuse
    let mut out2 = ByteBuffersDataOutput::new();
    Lz4::compress(data, offset, length, &mut out2, hash_table)?;
    assert_array_equals(&compressed, &out2.to_array_copy()?);

    // Now restore and compare bytes
    let mut restored = vec![0u8; length + random().next_int_bound(10) as usize];
    let mut restore_offset_end = Lz4::decompress(
        &mut ByteArrayDataInput::new(&compressed),
        length,
        &mut restored,
        0,
    )?;
    assert_equals(&length, &restore_offset_end);
    assert_array_equals(
        &sub(data, offset, offset + length),
        &sub(&restored, 0, length),
    );

    // Now restore with an offset
    let restore_offset = TestUtil::next_int(&*random(), 1, 10) as usize;
    restored = vec![0u8; restore_offset + length + random().next_int_bound(10) as usize];
    restore_offset_end = Lz4::decompress(
        &mut ByteArrayDataInput::new(&compressed),
        length,
        &mut restored,
        restore_offset,
    )?;
    assert_equals(&length, &(restore_offset_end - restore_offset));
    assert_array_equals(
        &sub(data, offset, offset + length),
        &sub(&restored, restore_offset, restore_offset + length),
    );
    Ok(())
}

/// `doTestWithDictionary(data, hashTable)`.
fn do_test_with_dictionary(data: &[u8], hash_table: &mut dyn Lz4HashTable) -> Result<()> {
    let mut copy = ByteBuffersDataOutput::new();
    let dict_off = TestUtil::next_int(&*random(), 0, 10) as usize;
    copy.write_bytes(&vec![0u8; dict_off], 0, dict_off)?;

    // Create a dictionary from substrings of the input to compress
    let mut dict_len = 0usize;
    let mut i = TestUtil::next_int(&*random(), 0, data.len() as i32) as usize;
    while i < data.len() && dict_len < LZ4_MAX_DISTANCE {
        let mut l = (data.len() - i).min(TestUtil::next_int(&*random(), 1, 32) as usize);
        l = l.min(LZ4_MAX_DISTANCE - dict_len);
        copy.write_bytes(data, i, l)?;
        dict_len += l;
        i += l;
        i += TestUtil::next_int(&*random(), 1, 32) as usize;
    }

    copy.write_bytes(data, 0, data.len())?;
    let pad = random().next_int_bound(10) as usize;
    copy.write_bytes(&vec![0u8; pad], 0, pad)?;

    let copy_bytes = copy.to_array_copy()?;
    do_test_with_dictionary_range(&copy_bytes, dict_off, dict_len, data.len(), hash_table)
}

/// `doTestWithDictionary(data, dictOff, dictLen, length, hashTable)`.
fn do_test_with_dictionary_range(
    data: &[u8],
    dict_off: usize,
    dict_len: usize,
    length: usize,
    hash_table: &mut dyn Lz4HashTable,
) -> Result<()> {
    let mut out = ByteBuffersDataOutput::new();
    Lz4::compress_with_dictionary(data, dict_off, dict_len, length, &mut out, hash_table)?;
    let compressed = out.to_array_copy()?;

    // Compress once again with the same hash table to test reuse
    let mut out2 = ByteBuffersDataOutput::new();
    Lz4::compress_with_dictionary(data, dict_off, dict_len, length, &mut out2, hash_table)?;
    assert_array_equals(&compressed, &out2.to_array_copy()?);

    // Now restore and compare bytes
    let restore_offset = TestUtil::next_int(&*random(), 1, 10) as usize;
    let mut restored =
        vec![0u8; restore_offset + dict_len + length + random().next_int_bound(10) as usize];
    restored[restore_offset..restore_offset + dict_len]
        .copy_from_slice(&data[dict_off..dict_off + dict_len]);
    Lz4::decompress(
        &mut ByteArrayDataInput::new(&compressed),
        length,
        &mut restored,
        dict_len + restore_offset,
    )?;
    assert_array_equals(
        &sub(data, dict_off + dict_len, dict_off + dict_len + length),
        &sub(
            &restored,
            restore_offset + dict_len,
            restore_offset + dict_len + length,
        ),
    );
    Ok(())
}

fn test_empty(new_hash_table: NewHashTable) -> Result<()> {
    let data = "".as_bytes();
    do_test(data, new_hash_table().as_mut())
}

fn test_short_literals_and_matchs(new_hash_table: NewHashTable) -> Result<()> {
    // literals and matchs lengths <= 15
    let data = "1234562345673456745678910123".as_bytes();
    do_test(data, new_hash_table().as_mut())?;
    do_test_with_dictionary(data, new_hash_table().as_mut())
}

fn test_long_matchs(new_hash_table: NewHashTable) -> Result<()> {
    // match length >= 20
    let mut data = vec![0u8; RandomNumbers::random_int_between(&*random(), 300, 1024) as usize];
    for (i, d) in data.iter_mut().enumerate() {
        *d = i as u8;
    }
    do_test(&data, new_hash_table().as_mut())
}

fn test_long_literals(new_hash_table: NewHashTable) -> Result<()> {
    // long literals (length >= 16) which are not the last literals
    let mut data = vec![0u8; RandomNumbers::random_int_between(&*random(), 400, 1024) as usize];
    random().next_bytes(&mut data);
    let match_ref = random().next_int_bound(30) as usize;
    let match_off = RandomNumbers::random_int_between(
        &*random(),
        data.len() as i32 - 40,
        data.len() as i32 - 20,
    ) as usize;
    let match_length = RandomNumbers::random_int_between(&*random(), 4, 10) as usize;
    data.copy_within(match_ref..match_ref + match_length, match_off);
    do_test(&data, new_hash_table().as_mut())
}

fn test_match_right_before_last_literals(new_hash_table: NewHashTable) -> Result<()> {
    do_test(
        &[1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 5],
        new_hash_table().as_mut(),
    )
}

fn test_incompressible_random(new_hash_table: NewHashTable) -> Result<()> {
    let mut b = vec![0u8; TestUtil::next_int(&*random(), 1, 1 << 18) as usize];
    random().next_bytes(&mut b);
    do_test(&b, new_hash_table().as_mut())?;
    do_test_with_dictionary(&b, new_hash_table().as_mut())
}

fn test_compressible_random(new_hash_table: NewHashTable) -> Result<()> {
    let mut b = vec![0u8; TestUtil::next_int(&*random(), 1, 1 << 18) as usize];
    let base = random().next_int_bound(256);
    let max_delta = 1 + random().next_int_bound(8);
    let r = random();
    for v in b.iter_mut() {
        *v = (base + r.next_int_bound(max_delta)) as u8;
    }
    do_test(&b, new_hash_table().as_mut())?;
    do_test_with_dictionary(&b, new_hash_table().as_mut())
}

/// The bytes of `testLUCENE5201`.
const LUCENE5201: &[i8] = &[
    14, 72, 14, 85, 3, 72, 14, 85, 3, 72, 14, 72, 14, 72, 14, 85, 3, 72, 14, 72, 14, 72, 14, 72,
    14, 72, 14, 72, 14, 85, 3, 72, 14, 85, 3, 72, 14, 85, 3, 72, 14, 85, 3, 72, 14, 85, 3, 72, 14,
    85, 3, 72, 14, 50, 64, 0, 46, -1, 0, 0, 0, 29, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, -97, 6,
    0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, -97,
    6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0,
    68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68,
    -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113,
    0, 50, 64, 0, 47, -105, 0, 0, 0, 30, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0,
    2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, 85, 8, -113, 0,
    68, -97, 3, 0, 2, -97, 6, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, -97, 6, 0, 68, -113, 0,
    2, 3, -97, 6, 0, 68, -113, 0, 120, 64, 0, 48, 4, 0, 0, 0, 31, 34, 72, 29, 72, 37, 72, 35, 72,
    45, 72, 23, 72, 46, 72, 20, 72, 40, 72, 33, 72, 25, 72, 39, 72, 38, 72, 26, 72, 28, 72, 42, 72,
    24, 72, 27, 72, 36, 72, 41, 72, 32, 72, 18, 72, 30, 72, 22, 72, 31, 72, 43, 72, 19, 72, 34, 72,
    29, 72, 37, 72, 35, 72, 45, 72, 23, 72, 46, 72, 20, 72, 40, 72, 33, 72, 25, 72, 39, 72, 38, 72,
    26, 72, 28, 72, 42, 72, 24, 72, 27, 72, 36, 72, 41, 72, 32, 72, 18, 72, 30, 72, 22, 72, 31, 72,
    43, 72, 19, 72, 34, 72, 29, 72, 37, 72, 35, 72, 45, 72, 23, 72, 46, 72, 20, 72, 40, 72, 33, 72,
    25, 72, 39, 72, 38, 72, 26, 72, 28, 72, 42, 72, 24, 72, 27, 72, 36, 72, 41, 72, 32, 72, 18, 16,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 39, 24, 32,
    34, 124, 0, 120, 64, 0, 48, 80, 0, 0, 0, 31, 30, 72, 22, 72, 31, 72, 43, 72, 19, 72, 34, 72,
    29, 72, 37, 72, 35, 72, 45, 72, 23, 72, 46, 72, 20, 72, 40, 72, 33, 72, 25, 72, 39, 72, 38, 72,
    26, 72, 28, 72, 42, 72, 24, 72, 27, 72, 36, 72, 41, 72, 32, 72, 18, 72, 30, 72, 22, 72, 31, 72,
    43, 72, 19, 72, 34, 72, 29, 72, 37, 72, 35, 72, 45, 72, 23, 72, 46, 72, 20, 72, 40, 72, 33, 72,
    25, 72, 39, 72, 38, 72, 26, 72, 28, 72, 42, 72, 24, 72, 27, 72, 36, 72, 41, 72, 32, 72, 18, 72,
    30, 72, 22, 72, 31, 72, 43, 72, 19, 72, 34, 72, 29, 72, 37, 72, 35, 72, 45, 72, 23, 72, 46, 72,
    20, 72, 40, 72, 33, 72, 25, 72, 39, 72, 38, 72, 26, 72, 28, 72, 42, 72, 24, 72, 27, 72, 36, 72,
    41, 72, 32, 72, 18, 72, 30, 72, 22, 72, 31, 72, 43, 72, 19, 72, 34, 72, 29, 72, 37, 72, 35, 72,
    45, 72, 23, 72, 46, 72, 20, 72, 40, 72, 33, 72, 25, 72, 39, 72, 38, 72, 26, 72, 28, 72, 42, 72,
    24, 72, 27, 72, 36, 72, 41, 72, 32, 72, 18, 72, 30, 72, 22, 72, 31, 72, 43, 72, 19, 72, 34, 72,
    29, 72, 37, 72, 35, 72, 45, 72, 23, 72, 46, 72, 20, 72, 40, 72, 33, 72, 25, 72, 39, 72, 38, 72,
    26, 72, 28, 72, 42, 72, 24, 72, 27, 72, 36, 72, 41, 72, 32, 72, 18, 72, 30, 72, 22, 72, 31, 72,
    43, 72, 19, 72, 34, 72, 29, 72, 37, 72, 35, 72, 45, 72, 23, 72, 46, 72, 20, 72, 40, 72, 33, 72,
    25, 72, 39, 72, 38, 72, 26, 72, 28, 72, 42, 72, 24, 72, 27, 72, 36, 72, 41, 72, 32, 72, 18, 72,
    30, 72, 22, 72, 31, 72, 43, 72, 19, 72, 34, 72, 29, 72, 37, 72, 35, 72, 45, 72, 23, 72, 46, 72,
    20, 72, 40, 72, 33, 72, 25, 72, 39, 72, 38, 72, 26, 72, 28, 72, 42, 72, 24, 72, 27, 72, 36, 72,
    41, 72, 32, 72, 18, 72, 30, 72, 22, 72, 31, 72, 43, 72, 19, 50, 64, 0, 49, 20, 0, 0, 0, 32, 3,
    -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2,
    3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0,
    2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3,
    -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97,
    6, 0, 50, 64, 0, 50, 53, 0, 0, 0, 34, 3, -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68,
    -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68,
    -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68,
    -97, 3, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, 85, 8,
    -113, 0, 68, -97, 3, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2,
    3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0,
    2, 3, -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, 85, 8, -113, 0, 68,
    -97, 3, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, -97, 6, 0, 50, 64, 0, 51, 85, 0, 0, 0,
    36, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113,
    0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, -97, 5, 0, 2, 3, 85, 8, -113,
    0, 68, -97, 3, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0,
    68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 50, -64, 0, 51, -45, 0, 0, 0, 37,
    68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68,
    -113, 0, 2, 3, -97, 6, 0, 68, -113, 0, 2, 3, 85, 8, -113, 0, 68, -113, 0, 2, 3, -97, 6, 0, 68,
    -113, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 2, 3, 85, 8, -113, 0, 68, -97, 3, 0, 120, 64, 0,
    52, -88, 0, 0, 0, 39, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 72, 13, 85, 5, 72, 13,
    85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 72, 13, 72,
    13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13,
    72, 13, 72, 13, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85,
    5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5,
    72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 85, 5, 72, 13, 72, 13, 72, 13, 72, 13, 85, 5, 72, 13, 85,
    5, 72, 13, 85, 5, 72, 13, 72, 13, 85, 5, 72, 13, 72, 13, 85, 5, 72, 13, 72, 13, 85, 5, 72, 13,
    -19, -24, -101, -35,
];

fn test_lucene5201(new_hash_table: NewHashTable) -> Result<()> {
    let data: Vec<u8> = LUCENE5201.iter().map(|&b| b as u8).collect();
    do_test_range(&data, 9, data.len() - 9, new_hash_table().as_mut())
}

fn test_use_dictionary(new_hash_table: NewHashTable) -> Result<()> {
    let b: [u8; 19] = [
        1, 2, 3, 4, 5, 6, // dictionary
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    ];
    let dict_off = 0;
    let dict_len = 6;
    let len = b.len() - dict_len;

    do_test_with_dictionary_range(&b, dict_off, dict_len, len, new_hash_table().as_mut())?;

    // The dictionary allows for better compression
    let mut out = ByteBuffersDataOutput::new();
    Lz4::compress_with_dictionary(
        &b,
        dict_off,
        dict_len,
        len,
        &mut out,
        new_hash_table().as_mut(),
    )?;
    Assert::assert_true(out.size() < len);
    Ok(())
}
