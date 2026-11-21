//! Ports of `lucene/core/src/test/org/apache/lucene/util/compress/TestFastLZ4.java`,
//! `TestHighLZ4.java` and `TestDecompressLZ4.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::store::ByteArrayDataInput;
use rucene::util::compress::{
    FastCompressionHashTable, HighCompressionHashTable, Lz4, Lz4HashTable,
};
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, test};
use crate::lz4_test_case::{self, AssertingHashTable};

/// `TestFastLZ4.newHashTable()`.
fn new_fast_hash_table() -> Box<dyn Lz4HashTable> {
    let hash_table: Box<dyn Lz4HashTable> = Box::new(FastCompressionHashTable::new());
    Box::new(AssertingHashTable::new(hash_table))
}

/// `TestHighLZ4.newHashTable()`.
fn new_high_hash_table() -> Box<dyn Lz4HashTable> {
    let hash_table: Box<dyn Lz4HashTable> = Box::new(HighCompressionHashTable::new());
    Box::new(AssertingHashTable::new(hash_table))
}

/// `TestFastLZ4 extends LZ4TestCase`.
pub fn fast_class_def() -> Arc<Class> {
    lz4_test_case::class_def(
        "org.apache.lucene.util.compress.TestFastLZ4",
        new_fast_hash_table,
    )
}

/// `TestHighLZ4 extends LZ4TestCase`.
pub fn high_class_def() -> Arc<Class> {
    lz4_test_case::class_def(
        "org.apache.lucene.util.compress.TestHighLZ4",
        new_high_hash_table,
    )
}

/// `TestDecompressLZ4`.
pub fn decompress_class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![test("testDecompressOffset0", test_decompress_offset0)];
    class("org.apache.lucene.util.compress.TestDecompressLZ4", methods)
}

fn test_decompress_offset0() -> Result<()> {
    let input: [u8; 11] = [
        0xE, // 0b_0000_1110 - token
        0,   // 0 offset - first byte
        0,   // 0 offset - second byte
        7 << 4,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    let mut output = [0u8; 18];
    let len = output.len();
    // `assertThrows(IOException.class, …)`.
    let e = expect_throws("java.io.IOException", || {
        Lz4::decompress(&mut ByteArrayDataInput::new(&input), len, &mut output, 0)
    });
    assert_equals(&Some("offset 0 is invalid"), &e.message());
    Ok(())
}
