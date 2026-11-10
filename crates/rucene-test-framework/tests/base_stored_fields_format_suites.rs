//! The `BaseStoredFieldsFormatTestCase` suites (rmp #351, #359): the
//! inherited cases of `BaseIndexFileFormatTestCase` and
//! `BaseStoredFieldsFormatTestCase` run, under the ported runner and
//! `LuceneTestCase`'s rule chain, on the codecs Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormat`
//!   (`lucene/core/src/test`): `TestUtil.getDefaultCodec()`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormatMergeInstance`
//!   (`lucene/core/src/test`): extends `TestLucene90StoredFieldsFormat`,
//!   overriding `shouldTestMergeInstance()` to return `true`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormatHighCompression`
//!   (`lucene/core/src/test`): `new Lucene104Codec(Mode.BEST_COMPRESSION)`;
//! * `org.apache.lucene.codecs.lucene90.compressing.TestCompressingStoredFieldsFormat`
//!   (`lucene/core/src/test`): `CompressingCodec.randomInstance(random())`
//!   when `TEST_NIGHTLY`, else `CompressingCodec.reasonableInstance(random())`;
//! * `org.apache.lucene.tests.codecs.asserting.TestAssertingStoredFieldsFormat`
//!   (`lucene/test-framework/src/test`, ported in full): `AssertingCodec`.
//!
//! The own test methods of the core classes (sprint 9) are declared on their
//! bindings; `TestLucene90StoredFieldsFormatMergeInstance` inherits those of
//! `TestLucene90StoredFieldsFormat`.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::lang::{double_to_string, float_to_string};
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene104::codec::{Lucene104Codec, Mode};
use rucene::codecs::lucene90::stored_fields::{
    Lucene90CompressingStoredFieldsReader, Lucene90CompressingStoredFieldsWriter,
    Lucene90StoredFieldsFormat,
};
use rucene::codecs::Codec;
use rucene::document::{Document, StoredField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{CodecReader, IndexReader};
use rucene::store::{ByteArrayDataInput, ByteArrayDataOutput, Directory};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::codecs::compressing::dummy::DummyCompressingCodec;
use rucene_test_framework::codecs::compressing::CompressingCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    format_test, rethrow, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_stored_fields_format_test_case::{
    self, BaseStoredFieldsFormatTestCase,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_seeds;
use suite_seeds::run_suite;

mod counting_prefetch;
use counting_prefetch::CountingPrefetchDirectory;

/// A stored-fields suite that overrides `getCodec()`, which `codec`
/// answers, and `shouldTestMergeInstance()` when `merge_instance` is set.
struct StoredFieldsSuite {
    codec: fn() -> Arc<dyn Codec>,
    merge_instance: bool,
}

impl BaseIndexFileFormatTestCase for StoredFieldsSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        (self.codec)()
    }

    fn should_test_merge_instance(&self) -> bool {
        self.merge_instance
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.stored_fields_add_random_fields(doc);
    }

    fn as_stored_fields(&self) -> Option<&dyn BaseStoredFieldsFormatTestCase> {
        Some(self)
    }
}

impl BaseStoredFieldsFormatTestCase for StoredFieldsSuite {}

/// The class `name`, extending `BaseStoredFieldsFormatTestCase`, whose
/// `getCodec()` is `codec` and whose own test methods are `methods`.
fn suite_class(name: &str, codec: fn() -> Arc<dyn Codec>, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&base_stored_fields_format_test_case::class())
        .constructor(Constructor::no_args(move || {
            FormatTestInstance::new(Arc::new(StoredFieldsSuite {
                codec,
                merge_instance: false,
            }))
        }));
    for method in methods {
        builder = builder.method(method);
    }
    builder.build()
}

/// `TestLucene90StoredFieldsFormat.getCodec()`: `TestUtil.getDefaultCodec()`.
fn lucene90_codec() -> Arc<dyn Codec> {
    TestUtil::get_default_codec()
}

/// `TestLucene90StoredFieldsFormat`.
fn lucene90_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormat",
        lucene90_codec,
        vec![format_test("testSkipRedundantPrefetches", |_| {
            test_skip_redundant_prefetches()
        })],
    )
}

/// Port of `TestLucene90StoredFieldsFormat.testSkipRedundantPrefetches`.
fn test_skip_redundant_prefetches() -> Result<()> {
    // Use the "dummy" codec, which has the same base class as Lucene90StoredFieldsFormat but allows
    // configuring the number of docs per chunk.
    let codec: Arc<dyn Codec> = Arc::new(DummyCompressingCodec::new(1 << 10, 2, false, 16)?);
    let mut orig_dir = LuceneTestCase::new_directory();
    let counter = Arc::new(AtomicI32::new(0));
    let dir: Arc<dyn Directory> = Arc::new(CountingPrefetchDirectory::new(
        orig_dir.as_directory(),
        Arc::clone(&counter),
    ));
    {
        let iwc = IndexWriterConfig::new();
        iwc.set_codec(codec)?;
        let w = IndexWriter::new(Arc::clone(&dir), iwc)?;
        for _ in 0..100 {
            let mut doc = Document::new();
            doc.add(Box::new(StoredField::new_string(
                "content",
                TestUtil::random_simple_string(&*LuceneTestCase::random()),
            )?));
            w.add_document(&doc)?;
        }
        w.force_merge(1)?;
        w.close()?;
    }

    {
        let reader = directory_reader::open(Arc::clone(&dir))?;
        let mut stored_fields = reader.stored_fields()?;
        counter.store(0, Ordering::SeqCst);
        Assert::assert_equals(0, counter.load(Ordering::SeqCst));
        stored_fields.prefetch(0)?;
        Assert::assert_equals(1, counter.load(Ordering::SeqCst));
        stored_fields.prefetch(1)?;
        // This format has 2 docs per block, so the second prefetch is skipped
        Assert::assert_equals(1, counter.load(Ordering::SeqCst));
        stored_fields.prefetch(15)?;
        Assert::assert_equals(2, counter.load(Ordering::SeqCst));
        stored_fields.prefetch(14)?;
        // 14 is in the same block as 15, so the prefetch was skipped
        Assert::assert_equals(2, counter.load(Ordering::SeqCst));
        // Already prefetched in the past, so skipped again
        stored_fields.prefetch(1)?;
        Assert::assert_equals(2, counter.load(Ordering::SeqCst));
        drop(stored_fields);
        reader.close()?;
    }
    drop(dir);
    orig_dir.close()
}

/// `TestLucene90StoredFieldsFormatMergeInstance`: `extends
/// TestLucene90StoredFieldsFormat` (whose `getCodec()` it inherits) and
/// overrides `shouldTestMergeInstance()` to return `true`.
fn merge_instance_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormatMergeInstance")
        .extends(&lucene90_class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(StoredFieldsSuite {
                codec: lucene90_codec,
                merge_instance: true,
            }))
        }))
        .build()
}

/// `TestLucene90StoredFieldsFormatHighCompression`: `getCodec()` is
/// `new Lucene104Codec(Lucene104Codec.Mode.BEST_COMPRESSION)`.
fn high_compression_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormatHighCompression",
        || {
            Arc::new(
                Lucene104Codec::with_mode(Some(Mode::BestCompression))
                    .unwrap_or_else(|e| rethrow(e)),
            )
        },
        vec![
            format_test("testMixedCompressions", |_| test_mixed_compressions()),
            format_test("testInvalidOptions", |_| test_invalid_options()),
        ],
    )
}

/// Change compression params (leaving it the same for old segments) and tests that nothing breaks.
///
/// Port of `TestLucene90StoredFieldsFormatHighCompression.testMixedCompressions`.
fn test_mixed_compressions() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    for _ in 0..10 {
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_codec(Arc::new(Lucene104Codec::with_mode(Some(
            *RandomPicks::random_from(
                &*LuceneTestCase::random(),
                &[Mode::BestSpeed, Mode::BestCompression],
            ),
        ))?))?;
        // As in Java, `iwc` is not the config the writer receives.
        drop(iwc);
        let iw = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let mut doc = Document::new();
        doc.add(Box::new(StoredField::new_string(
            "field1",
            "value1".to_string(),
        )?));
        doc.add(Box::new(StoredField::new_string(
            "field2",
            "value2".to_string(),
        )?));
        iw.add_document(&doc)?;
        if LuceneTestCase::random().next_int_bound(4) == 0 {
            iw.force_merge(1)?;
        }
        iw.commit()?;
        iw.close()?;
    }

    let ir = directory_reader::open(dir.as_directory())?;
    Assert::assert_equals(10, ir.num_docs());
    let stored_fields = ir.stored_fields()?;
    for i in 0..10 {
        let doc = stored_fields.document(i)?;
        Assert::assert_equals_str(None, Some("value1"), doc.get("field1").as_deref());
        Assert::assert_equals_str(None, Some("value2"), doc.get("field2").as_deref());
    }
    drop(stored_fields);
    ir.close()?;
    // checkindex
    dir.close()
}

/// Port of `TestLucene90StoredFieldsFormatHighCompression.testInvalidOptions`.
fn test_invalid_options() -> Result<()> {
    expect_throws("java.lang.NullPointerException", || {
        Lucene104Codec::with_mode(None)
    });

    expect_throws("java.lang.NullPointerException", || {
        Lucene90StoredFieldsFormat::with_mode(None)
    });
    Ok(())
}

/// `TestCompressingStoredFieldsFormat.SECOND`.
const SECOND: i64 = 1000;
/// `TestCompressingStoredFieldsFormat.HOUR`.
const HOUR: i64 = 60 * 60 * SECOND;
/// `TestCompressingStoredFieldsFormat.DAY`.
const DAY: i64 = 24 * HOUR;

/// `TestCompressingStoredFieldsFormat`: `getCodec()` is
/// `CompressingCodec.randomInstance(random())` when `TEST_NIGHTLY`, else
/// `CompressingCodec.reasonableInstance(random())`.
fn compressing_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.compressing.TestCompressingStoredFieldsFormat",
        || {
            let codec = if *TEST_NIGHTLY {
                CompressingCodec::random_instance(&*LuceneTestCase::random())
            } else {
                CompressingCodec::reasonable_instance(&*LuceneTestCase::random())
            };
            Arc::new(codec.unwrap_or_else(|e| rethrow(e)))
        },
        vec![
            format_test("testZFloat", |_| test_z_float()),
            format_test("testZDouble", |_| test_z_double()),
            format_test("testTLong", |_| test_t_long()),
            format_test("testChunkCleanup", |_| test_chunk_cleanup()),
        ],
    )
}

/// `Float.floatToIntBits(f)`: the bits, every NaN collapsed to the canonical
/// one.
fn float_to_int_bits(f: f32) -> i32 {
    if f.is_nan() {
        0x7fc0_0000
    } else {
        f.to_bits() as i32
    }
}

/// `Double.doubleToLongBits(d)`: the bits, every NaN collapsed to the
/// canonical one.
fn double_to_long_bits(d: f64) -> i64 {
    if d.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        d.to_bits() as i64
    }
}

// Java shares one `buffer` between `out` and `in`; Rust cannot hold a mutable
// and a shared borrow of it at once, so each round trip writes through an
// output over the buffer, then reads through an input over it.
// `out.reset(buffer)` is the fresh output of the next round.

/// `writeZFloat` into `buffer`, then `readZFloat` back: the number of bytes
/// written, the value read and `in.eof()`.
fn round_trip_z_float(buffer: &mut [u8], f: f32) -> Result<(usize, f32, bool)> {
    let mut out = ByteArrayDataOutput::new(buffer);
    Lucene90CompressingStoredFieldsWriter::write_z_float(&mut out, f)?;
    let position = out.position();
    let mut input = ByteArrayDataInput::with_range(buffer, 0, position);
    let g = Lucene90CompressingStoredFieldsReader::read_z_float(&mut input)?;
    Ok((position, g, input.eof()))
}

/// `writeZDouble` into `buffer`, then `readZDouble` back.
fn round_trip_z_double(buffer: &mut [u8], x: f64) -> Result<(usize, f64, bool)> {
    let mut out = ByteArrayDataOutput::new(buffer);
    Lucene90CompressingStoredFieldsWriter::write_z_double(&mut out, x)?;
    let position = out.position();
    let mut input = ByteArrayDataInput::with_range(buffer, 0, position);
    let y = Lucene90CompressingStoredFieldsReader::read_z_double(&mut input)?;
    Ok((position, y, input.eof()))
}

/// `writeTLong` into `buffer`, then `readTLong` back.
fn round_trip_t_long(buffer: &mut [u8], l1: i64) -> Result<(usize, i64, bool)> {
    let mut out = ByteArrayDataOutput::new(buffer);
    Lucene90CompressingStoredFieldsWriter::write_t_long(&mut out, l1)?;
    let position = out.position();
    let mut input = ByteArrayDataInput::with_range(buffer, 0, position);
    let l2 = Lucene90CompressingStoredFieldsReader::read_t_long(&mut input)?;
    Ok((position, l2, input.eof()))
}

/// Port of `TestCompressingStoredFieldsFormat.testZFloat`.
fn test_z_float() -> Result<()> {
    let mut buffer = [0u8; 5]; // we never need more than 5 bytes

    // round-trip small integer values
    for i in i32::from(i16::MIN)..i32::from(i16::MAX) {
        let f = i as f32;
        let (position, g, eof) = round_trip_z_float(&mut buffer, f)?;
        Assert::assert_true(eof);
        Assert::assert_equals(float_to_int_bits(f), float_to_int_bits(g));

        // check that compression actually works
        if (-1..=123).contains(&i) {
            Assert::assert_equals(1, position); // single byte compression
        }
    }

    // round-trip special values
    let special = [
        -0.0f32,
        0.0f32,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::from_bits(1), // Float.MIN_VALUE
        f32::MAX,
        f32::NAN,
    ];

    for f in special {
        let (_, g, eof) = round_trip_z_float(&mut buffer, f)?;
        Assert::assert_true(eof);
        Assert::assert_equals(float_to_int_bits(f), float_to_int_bits(g));
    }

    // round-trip random values
    let r = LuceneTestCase::random();
    for _ in 0..100000 {
        let f = r.next_float() * (LuceneTestCase::random().next_int_bound(100) - 50) as f32;
        let mut out = ByteArrayDataOutput::new(&mut buffer);
        Lucene90CompressingStoredFieldsWriter::write_z_float(&mut out, f)?;
        let position = out.position();
        Assert::assert_true_msg(
            &format!("length={position}, f={}", float_to_string(f)),
            position
                <= if ((float_to_int_bits(f) as u32) >> 31) == 1 {
                    5
                } else {
                    4
                },
        );
        let mut input = ByteArrayDataInput::with_range(&buffer, 0, position);
        let g = Lucene90CompressingStoredFieldsReader::read_z_float(&mut input)?;
        Assert::assert_true(input.eof());
        Assert::assert_equals(float_to_int_bits(f), float_to_int_bits(g));
    }
    Ok(())
}

/// Port of `TestCompressingStoredFieldsFormat.testZDouble`.
fn test_z_double() -> Result<()> {
    let mut buffer = [0u8; 9]; // we never need more than 9 bytes

    // round-trip small integer values
    for i in i32::from(i16::MIN)..i32::from(i16::MAX) {
        let x = f64::from(i);
        let (position, y, eof) = round_trip_z_double(&mut buffer, x)?;
        Assert::assert_true(eof);
        Assert::assert_equals(double_to_long_bits(x), double_to_long_bits(y));

        // check that compression actually works
        if (-1..=124).contains(&i) {
            Assert::assert_equals(1, position); // single byte compression
        }
    }

    // round-trip special values
    let special = [
        -0.0f64,
        0.0f64,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::from_bits(1), // Double.MIN_VALUE
        f64::MAX,
        f64::NAN,
    ];

    for x in special {
        let (_, y, eof) = round_trip_z_double(&mut buffer, x)?;
        Assert::assert_true(eof);
        Assert::assert_equals(double_to_long_bits(x), double_to_long_bits(y));
    }

    // round-trip random values
    let r = LuceneTestCase::random();
    for _ in 0..100000 {
        let x = r.next_double() * f64::from(LuceneTestCase::random().next_int_bound(100) - 50);
        let mut out = ByteArrayDataOutput::new(&mut buffer);
        Lucene90CompressingStoredFieldsWriter::write_z_double(&mut out, x)?;
        let position = out.position();
        Assert::assert_true_msg(
            &format!("length={position}, d={}", double_to_string(x)),
            position <= if x < 0.0 { 9 } else { 8 },
        );
        let mut input = ByteArrayDataInput::with_range(&buffer, 0, position);
        let y = Lucene90CompressingStoredFieldsReader::read_z_double(&mut input)?;
        Assert::assert_true(input.eof());
        Assert::assert_equals(double_to_long_bits(x), double_to_long_bits(y));
    }

    // same with floats
    for _ in 0..100000 {
        let x =
            f64::from(r.next_float() * (LuceneTestCase::random().next_int_bound(100) - 50) as f32);
        let mut out = ByteArrayDataOutput::new(&mut buffer);
        Lucene90CompressingStoredFieldsWriter::write_z_double(&mut out, x)?;
        let position = out.position();
        Assert::assert_true_msg(
            &format!("length={position}, d={}", double_to_string(x)),
            position <= 5,
        );
        let mut input = ByteArrayDataInput::with_range(&buffer, 0, position);
        let y = Lucene90CompressingStoredFieldsReader::read_z_double(&mut input)?;
        Assert::assert_true(input.eof());
        Assert::assert_equals(double_to_long_bits(x), double_to_long_bits(y));
    }
    Ok(())
}

/// Port of `TestCompressingStoredFieldsFormat.testTLong`.
fn test_t_long() -> Result<()> {
    let mut buffer = [0u8; 10]; // we never need more than 10 bytes

    // round-trip small integer values
    for i in i32::from(i16::MIN)..i32::from(i16::MAX) {
        for mul in [SECOND, HOUR, DAY] {
            let l1 = i64::from(i) * mul;
            let (position, l2, eof) = round_trip_t_long(&mut buffer, l1)?;
            Assert::assert_true(eof);
            Assert::assert_equals(l1, l2);

            // check that compression actually works
            if (-16..=15).contains(&i) {
                Assert::assert_equals(1, position); // single byte compression
            }
        }
    }

    // round-trip random values
    let r = LuceneTestCase::random();
    for _ in 0..100000 {
        let num_bits = r.next_int_bound(65);
        // Java's `1L << 64` is `1L << 0`: the mask of 64 bits is 0.
        let mut l1 = r.next_long() & 1i64.wrapping_shl(num_bits as u32).wrapping_sub(1);
        match r.next_int_bound(4) {
            0 => l1 = l1.wrapping_mul(SECOND),
            1 => l1 = l1.wrapping_mul(HOUR),
            2 => l1 = l1.wrapping_mul(DAY),
            _ => {}
        }
        let (_, l2, eof) = round_trip_t_long(&mut buffer, l1)?;
        Assert::assert_true(eof);
        Assert::assert_equals(l1, l2);
    }
    Ok(())
}

/// `(Lucene90CompressingStoredFieldsReader) ((CodecReader) reader).getFieldsReader()`:
/// the casts' `ClassCastException`, or the `NullPointerException` of the
/// next call on a `null` reader.
fn with_compressing_fields_reader<T>(
    reader: &Arc<dyn rucene::index::LeafReader>,
    body: impl FnOnce(&Lucene90CompressingStoredFieldsReader) -> T,
) -> Result<T> {
    let sr = Arc::clone(reader).as_codec_reader().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.ClassCastException",
            "the leaf reader is not a CodecReader",
        )
        .throw()
    });
    let fields_reader = CodecReader::get_fields_reader(&*sr)?.unwrap_or_else(|| {
        Throwable::new(
            "java.lang.NullPointerException",
            "getFieldsReader() is null",
        )
        .throw()
    });
    let compressing = fields_reader
        .as_any()
        .and_then(|any| any.downcast_ref::<Lucene90CompressingStoredFieldsReader>())
        .unwrap_or_else(|| {
            Throwable::new(
                "java.lang.ClassCastException",
                "the fields reader is not a Lucene90CompressingStoredFieldsReader",
            )
            .throw()
        });
    Ok(body(compressing))
}

/// writes some tiny segments with incomplete compressed blocks, and ensures merge recompresses
/// them.
///
/// Port of `TestCompressingStoredFieldsFormat.testChunkCleanup`.
fn test_chunk_cleanup() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw_conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*LuceneTestCase::random()),
    )));
    iw_conf.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;

    // we have to enforce certain things like maxDocsPerChunk to cause dirty chunks to be created
    // by this test.
    iw_conf.set_codec(Arc::new(CompressingCodec::random_instance_with(
        &*LuceneTestCase::random(),
        4 * 1024,
        4,
        false,
        8,
    )?))?;
    let iw = IndexWriter::new(dir.as_directory(), iw_conf)?;
    let mut ir = directory_reader::open_with_writer(&*iw)?;
    for _ in 0..5 {
        let mut doc = Document::new();
        doc.add(Box::new(StoredField::new_string(
            "text",
            "not very long at all".to_string(),
        )?));
        iw.add_document(&doc)?;
        // force flush
        let ir2 = directory_reader::open_if_changed(Arc::clone(&ir))?;
        Assert::assert_true(ir2.is_some()); // assertNotNull(ir2)
        let ir2 = ir2.expect("assertNotNull(ir2)");
        ir.close()?;
        ir = ir2;
        // examine dirty counts:
        for leaf in Arc::clone(&ir).leaves()? {
            with_compressing_fields_reader(&leaf.leaf_reader(), |reader| {
                Assert::assert_true(reader.num_dirty_docs() > 0);
                Assert::assert_true(reader.num_dirty_docs() < 100); // can't be gte the number of docs per chunk
                Assert::assert_equals(1, reader.num_dirty_chunks());
            })?;
        }
    }
    iw.get_config()
        .set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    iw.force_merge(1)?;
    // add a single doc and merge again
    let mut doc = Document::new();
    doc.add(Box::new(StoredField::new_string(
        "text",
        "not very long at all".to_string(),
    )?));
    iw.add_document(&doc)?;
    iw.force_merge(1)?;
    let ir2 = directory_reader::open_if_changed(Arc::clone(&ir))?;
    Assert::assert_true(ir2.is_some()); // assertNotNull(ir2)
    let ir2 = ir2.expect("assertNotNull(ir2)");
    ir.close()?;
    ir = ir2;
    let sr = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as Arc<dyn IndexReader>)?;
    with_compressing_fields_reader(&sr, |reader| {
        // at most 2: the 5 chunks from 5 doc segment will be collapsed into a single chunk
        Assert::assert_true(reader.num_dirty_chunks() <= 2);
    })?;
    ir.close()?;
    iw.close()?;
    dir.close()
}

/// `TestAssertingStoredFieldsFormat`: `private final Codec codec = new
/// AssertingCodec()`.
struct TestAssertingStoredFieldsFormat {
    codec: Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for TestAssertingStoredFieldsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.stored_fields_add_random_fields(doc);
    }

    fn as_stored_fields(&self) -> Option<&dyn BaseStoredFieldsFormatTestCase> {
        Some(self)
    }
}

impl BaseStoredFieldsFormatTestCase for TestAssertingStoredFieldsFormat {}

fn asserting_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.codecs.asserting.TestAssertingStoredFieldsFormat")
        .extends(&base_stored_fields_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestAssertingStoredFieldsFormat {
                codec: Arc::new(AssertingCodec::new()),
            }))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestStoredFieldsSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestStoredFieldsSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestStoredFieldsSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90StoredFieldsFormat`, recording the next
/// `random().nextLong()` after each test and after the class.
fn draws_class() -> Arc<Class> {
    use randomizedtesting::jdk::reflect::modifier;
    use rucene_test_framework::index::base_index_file_format_test_case::tear_down;
    Class::builder("org.apache.lucene.tests.index.TestStoredFieldsSuiteDraws")
        .extends(&base_stored_fields_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(StoredFieldsSuite {
                codec: lucene90_codec,
                merge_instance: false,
            }))
        }))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut FormatTestInstance| {
                    DRAWS.lock().unwrap().push(format!(
                        "{}\tnext\t{}",
                        LuceneTestCase::get_test_name().unwrap_or_default(),
                        LuceneTestCase::random().next_long()
                    ));
                    tear_down(inst);
                }),
        )
        .method(
            Method::new("recordClassDraw")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(randomizedtesting::junit::AfterClass)
                .static_body(|| {
                    DRAWS.lock().unwrap().push(format!(
                        "#class\tnext\t{}",
                        LuceneTestCase::random().next_long()
                    ));
                }),
        )
        .build()
}

/// The rows of one seed of `TestStoredFieldsSuiteDraws`, as
/// `BaseFormatSuitesOracle` writes them (sorted).
fn draw_rows(seed: &str) -> Vec<String> {
    use randomizedtesting::jdk::system;
    use randomizedtesting::junit::{Failure, JUnitCore, Request, RunListener};
    struct Events(Mutex<Vec<String>>);
    impl RunListener for Events {
        fn test_failure(&self, f: &Failure) {
            let m = f
                .description()
                .method_name()
                .unwrap_or_else(|| "null".into());
            let t = f.exception();
            let first = t
                .message()
                .unwrap_or("null")
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            self.0
                .lock()
                .unwrap()
                .push(format!("{m}\tF\t{}: {first}", t.class_name()));
        }
        fn test_assumption_failure(&self, f: &Failure) {
            let m = f
                .description()
                .method_name()
                .unwrap_or_else(|| "null".into());
            let first = f
                .message()
                .unwrap_or("null")
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            self.0.lock().unwrap().push(format!("{m}\tA\t{first}"));
        }
    }
    system::set_property("tests.seed", seed);
    DRAWS.lock().unwrap().clear();
    let events = Arc::new(Events(Mutex::new(Vec::new())));
    let mut core = JUnitCore::new();
    core.add_listener(Arc::clone(&events) as Arc<dyn RunListener>);
    let r = core.run(&Request::a_class(&draws_class()));
    system::clear_property("tests.seed");
    let mut rows: Vec<String> = DRAWS.lock().unwrap().clone();
    rows.extend(events.0.lock().unwrap().iter().cloned());
    rows.push(format!(
        "#counts\trun={} fail={} assume={}\t-",
        r.run_count(),
        r.failure_count(),
        r.assumption_failure_count()
    ));
    rows.sort();
    rows.into_iter()
        .map(|row| format!("{seed}\t{row}"))
        .collect()
}

/// Seeded-draw parity with Lucene (rmp #351): on the oracle's seeds, every
/// test of the suite leaves its randomness where Java's does, and the class
/// leaves its own where Java's does (`base_stored_fields_draws.tsv`,
/// `BaseFormatSuitesOracle`): every row must be equal.
#[test]
fn test_stored_fields_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    let java: Vec<String> = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/base_stored_fields_draws.tsv"
    ))
    .unwrap()
    .lines()
    .filter(|l| !l.starts_with('#'))
    .map(str::to_string)
    .collect();
    let mut seeds: Vec<String> = Vec::new();
    for row in &java {
        let seed = row.split('\t').next().unwrap().to_string();
        if !seeds.contains(&seed) {
            seeds.push(seed);
        }
    }
    let mut rust: Vec<String> = Vec::new();
    for seed in &seeds {
        rust.extend(draw_rows(seed));
    }
    let only_java: Vec<&String> = java.iter().filter(|r| !rust.contains(r)).collect();
    let only_rust: Vec<&String> = rust.iter().filter(|r| !java.contains(r)).collect();
    println!(
        "draw rows: java={} rust={} equal={}",
        java.len(),
        rust.len(),
        java.len() - only_java.len()
    );
    for r in &only_java {
        println!("  java only: {r}");
    }
    for r in &only_rust {
        println!("  rust only: {r}");
    }
    assert!(
        only_java.is_empty() && only_rust.is_empty(),
        "rows differ from Lucene's: java only {only_java:?}, rust only {only_rust:?}"
    );
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_lucene90_stored_fields_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}

#[test]
fn test_lucene90_stored_fields_format_merge_instance() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(merge_instance_class);
}

#[test]
fn test_lucene90_stored_fields_format_high_compression() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(high_compression_class);
}

#[test]
fn test_compressing_stored_fields_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(compressing_class);
}

#[test]
fn test_asserting_stored_fields_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(asserting_class);
}
