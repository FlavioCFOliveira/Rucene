//! Port of `lucene/core/src/test/org/apache/lucene/codecs/compressing/AbstractTestCompressionMode.java`
//! and of its five subclasses `TestDeflateWithPresetDictCompressionMode`,
//! `TestFastCompressionMode`, `TestFastDecompressionMode`,
//! `TestHighCompressionMode` and `TestLZ4WithPresetDictCompressionMode`,
//! whose only member is the `setUp` that picks the `mode`.
//!
//! The abstract base is the [`CompressionModeTest`] fixture (its `mode`
//! field) and the shared test bodies; each subclass is a class binding all
//! of the inherited test methods with its own `setUp`.
//!
//! Java's `compress` hands the compressor a `ByteBuffersDataInput` slice of
//! the input; the port's `Compressor.compress` reads a
//! `ByteBuffersDataOutput`, so the slice `[off, off + len)` is written to one.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::compressing::{
    CompressionMode, Compressor, Decompressor, DeflateWithPresetDictCompressionMode,
    LZ4WithPresetDictCompressionMode, FAST, FAST_DECOMPRESSION, HIGH_COMPRESSION,
};
use rucene::error::Result;
use rucene::store::{ByteArrayDataInput, ByteArrayDataOutput, ByteBuffersDataOutput, DataOutput};
use rucene::util::BytesRef;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_codecs::support::{fixture_class, ftest, set_up};
use crate::core_support::{assert_array_equals, assert_equals, random};

/// The instance of `AbstractTestCompressionMode`: `CompressionMode mode`.
#[derive(Default)]
pub struct CompressionModeTest {
    mode: Option<Arc<dyn CompressionMode>>,
}

impl CompressionModeTest {
    fn mode(&self) -> &Arc<dyn CompressionMode> {
        self.mode.as_ref().expect("setUp ran")
    }
}

/// The class of a subclass of `AbstractTestCompressionMode`, with its
/// `setUp` and the inherited test methods.
fn compression_mode_class(
    name: &str,
    set_up_body: fn(&mut CompressionModeTest) -> Result<()>,
) -> Arc<Class> {
    fixture_class::<CompressionModeTest>(
        name,
        vec![
            set_up(set_up_body),
            ftest("testDecompress", test_decompress),
            ftest("testPartialDecompress", test_partial_decompress),
            ftest("testEmptySequence", test_empty_sequence),
            ftest("testShortSequence", test_short_sequence),
            ftest("testIncompressible", test_incompressible),
            ftest("testConstant", test_constant),
            ftest("testExtremelyLargeInput", test_extremely_large_input),
        ],
    )
}

/// `TestDeflateWithPresetDictCompressionMode`.
pub fn deflate_with_preset_dict_class_def() -> Arc<Class> {
    compression_mode_class(
        "org.apache.lucene.codecs.compressing.TestDeflateWithPresetDictCompressionMode",
        |f| {
            f.mode = Some(Arc::new(DeflateWithPresetDictCompressionMode::new()));
            Ok(())
        },
    )
}

/// `TestFastCompressionMode`.
pub fn fast_class_def() -> Arc<Class> {
    compression_mode_class(
        "org.apache.lucene.codecs.compressing.TestFastCompressionMode",
        |f| {
            f.mode = Some(Arc::clone(&*FAST));
            Ok(())
        },
    )
}

/// `TestFastDecompressionMode`.
pub fn fast_decompression_class_def() -> Arc<Class> {
    compression_mode_class(
        "org.apache.lucene.codecs.compressing.TestFastDecompressionMode",
        |f| {
            f.mode = Some(Arc::clone(&*FAST_DECOMPRESSION));
            Ok(())
        },
    )
}

/// `TestHighCompressionMode`.
pub fn high_compression_class_def() -> Arc<Class> {
    compression_mode_class(
        "org.apache.lucene.codecs.compressing.TestHighCompressionMode",
        |f| {
            f.mode = Some(Arc::clone(&*HIGH_COMPRESSION));
            Ok(())
        },
    )
}

/// `TestLZ4WithPresetDictCompressionMode`.
pub fn lz4_with_preset_dict_class_def() -> Arc<Class> {
    compression_mode_class(
        "org.apache.lucene.codecs.compressing.TestLZ4WithPresetDictCompressionMode",
        |f| {
            f.mode = Some(Arc::new(LZ4WithPresetDictCompressionMode::new()));
            Ok(())
        },
    )
}

/// `static byte[] randomArray(Random random)`.
pub fn random_array(random: &dyn Random) -> Vec<u8> {
    let bigsize = if *TEST_NIGHTLY { 192 * 1024 } else { 33 * 1024 };
    let max = if random.next_boolean() {
        random.next_int_bound(4)
    } else {
        random.next_int_bound(255)
    };
    let length = if random.next_boolean() {
        random.next_int_bound(20)
    } else {
        random.next_int_bound(bigsize)
    };
    random_array_with(random, length, max)
}

/// `static byte[] randomArray(Random random, int length, int max)`.
pub fn random_array_with(random: &dyn Random, length: i32, max: i32) -> Vec<u8> {
    let mut arr = vec![0u8; length as usize];
    for i in 0..arr.len() {
        arr[i] = RandomNumbers::random_int_between(random, 0, max) as u8;
    }
    arr
}

/// `byte[] compress(byte[] decompressed, int off, int len)`.
fn compress(
    f: &CompressionModeTest,
    decompressed: &[u8],
    off: usize,
    len: usize,
) -> Result<Vec<u8>> {
    let mut compressor = f.mode().new_compressor();
    compress_with(&mut *compressor, decompressed, off, len)
}

/// `static byte[] compress(Compressor compressor, byte[] decompressed, int off, int len)`.
pub fn compress_with(
    compressor: &mut dyn Compressor,
    decompressed: &[u8],
    off: usize,
    len: usize,
) -> Result<Vec<u8>> {
    let mut compressed = vec![0u8; len * 3 + 16]; // should be enough
    let mut input = ByteBuffersDataOutput::new();
    input.write_bytes(decompressed, off, len)?;

    let compressed_len = {
        let mut out = ByteArrayDataOutput::new(&mut compressed);

        compressor.compress(&input, &mut out)?;
        out.position()
    };
    Ok(compressed[..compressed_len].to_vec())
}

/// `byte[] decompress(byte[] compressed, int originalLength)`.
fn decompress(
    f: &CompressionModeTest,
    compressed: &[u8],
    original_length: usize,
) -> Result<Vec<u8>> {
    let mut decompressor = f.mode().new_decompressor();
    decompress_with(&mut *decompressor, compressed, original_length)
}

/// `static byte[] decompress(Decompressor decompressor, byte[] compressed, int originalLength)`.
pub fn decompress_with(
    decompressor: &mut dyn Decompressor,
    compressed: &[u8],
    original_length: usize,
) -> Result<Vec<u8>> {
    let mut bytes = BytesRef::default();
    decompressor.decompress(
        &mut ByteArrayDataInput::new(compressed),
        original_length,
        0,
        original_length,
        &mut bytes,
    )?;
    Ok(BytesRef::deep_copy_of(&bytes).bytes)
}

/// `byte[] decompress(byte[] compressed, int originalLength, int offset, int length)`.
fn decompress_range(
    f: &CompressionModeTest,
    compressed: &[u8],
    original_length: usize,
    offset: usize,
    length: usize,
) -> Result<Vec<u8>> {
    let mut decompressor = f.mode().new_decompressor();
    let mut bytes = BytesRef::default();
    decompressor.decompress(
        &mut ByteArrayDataInput::new(compressed),
        original_length,
        offset,
        length,
        &mut bytes,
    )?;
    Ok(BytesRef::deep_copy_of(&bytes).bytes)
}

/// Port of `AbstractTestCompressionMode.testDecompress`.
fn test_decompress(f: &mut CompressionModeTest) -> Result<()> {
    let random = random();
    let iterations = LuceneTestCase::at_least_with(&*random, 3);
    for _i in 0..iterations {
        let decompressed = random_array(&*random);
        let off = if random.next_boolean() {
            0
        } else {
            TestUtil::next_int(&*random, 0, decompressed.len() as i32)
        } as usize;
        let len = if random.next_boolean() {
            decompressed.len() - off
        } else {
            TestUtil::next_int(&*random, 0, (decompressed.len() - off) as i32) as usize
        };
        let compressed = compress(f, &decompressed, off, len)?;
        let restored = decompress(f, &compressed, len)?;
        assert_array_equals(&decompressed[off..off + len], &restored);
    }
    Ok(())
}

/// Port of `AbstractTestCompressionMode.testPartialDecompress`.
fn test_partial_decompress(f: &mut CompressionModeTest) -> Result<()> {
    let random = random();
    let iterations = LuceneTestCase::at_least_with(&*random, 3);
    for _i in 0..iterations {
        let decompressed = random_array(&*random);
        let compressed = compress(f, &decompressed, 0, decompressed.len())?;
        let (offset, length) = if decompressed.is_empty() {
            (0, 0)
        } else {
            let offset = random.next_int_bound(decompressed.len() as i32) as usize;
            let length = random.next_int_bound((decompressed.len() - offset) as i32) as usize;
            (offset, length)
        };
        let restored = decompress_range(f, &compressed, decompressed.len(), offset, length)?;
        assert_array_equals(&decompressed[offset..offset + length], &restored);
    }
    Ok(())
}

/// `public byte[] test(byte[] decompressed)`.
fn test_bytes(f: &CompressionModeTest, decompressed: &[u8]) -> Result<Vec<u8>> {
    test_range(f, decompressed, 0, decompressed.len())
}

/// `public byte[] test(byte[] decompressed, int off, int len)`.
fn test_range(
    f: &CompressionModeTest,
    decompressed: &[u8],
    off: usize,
    len: usize,
) -> Result<Vec<u8>> {
    let compressed = compress(f, decompressed, off, len)?;
    let restored = decompress(f, &compressed, len)?;
    assert_equals(&len, &restored.len());
    Ok(compressed)
}

/// Port of `AbstractTestCompressionMode.testEmptySequence`.
fn test_empty_sequence(f: &mut CompressionModeTest) -> Result<()> {
    test_bytes(f, &[])?;
    Ok(())
}

/// Port of `AbstractTestCompressionMode.testShortSequence`.
fn test_short_sequence(f: &mut CompressionModeTest) -> Result<()> {
    test_bytes(f, &[random().next_int_bound(256) as u8])?;
    Ok(())
}

/// Port of `AbstractTestCompressionMode.testIncompressible`.
fn test_incompressible(f: &mut CompressionModeTest) -> Result<()> {
    let mut decompressed =
        vec![0u8; RandomNumbers::random_int_between(&*random(), 20, 256) as usize];
    for i in 0..decompressed.len() {
        decompressed[i] = i as u8;
    }
    test_bytes(f, &decompressed)?;
    Ok(())
}

/// Port of `AbstractTestCompressionMode.testConstant`.
fn test_constant(f: &mut CompressionModeTest) -> Result<()> {
    let mut decompressed = vec![0u8; TestUtil::next_int(&*random(), 1, 10000) as usize];
    decompressed.fill(random().next_int() as u8);
    test_bytes(f, &decompressed)?;
    Ok(())
}

/// Port of `AbstractTestCompressionMode.testExtremelyLargeInput`.
fn test_extremely_large_input(f: &mut CompressionModeTest) -> Result<()> {
    let mut decompressed = vec![0u8; 1 << 24]; // 16MB
    for i in 0..decompressed.len() {
        decompressed[i] = (i & 0x0F) as u8;
    }
    test_bytes(f, &decompressed)?;
    Ok(())
}
