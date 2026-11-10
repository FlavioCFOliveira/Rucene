//! The `BaseTermVectorsFormatTestCase` suites (rmp #352, #359): the
//! inherited cases of `BaseIndexFileFormatTestCase` and
//! `BaseTermVectorsFormatTestCase` run, under the ported runner and
//! `LuceneTestCase`'s rule chain, on the codecs Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90TermVectorsFormat`
//!   (`lucene/core/src/test`): `getCodec()` returns
//!   `TestUtil.getDefaultCodec()`;
//! * `org.apache.lucene.codecs.lucene90.compressing.TestCompressingTermVectorsFormat`
//!   (`lucene/core/src/test`): `getCodec()` returns
//!   `CompressingCodec.randomInstance(random())` when `TEST_NIGHTLY`, else
//!   `CompressingCodec.reasonableInstance(random())`;
//! * `org.apache.lucene.tests.codecs.asserting.TestAssertingTermVectorsFormat`
//!   (`lucene/test-framework/src/test`, ported in full): `AssertingCodec`,
//!   and `getReadPastLastPositionExceptionClass()` returns
//!   `AssertionError.class`.
//!
//! `org.apache.lucene.index.TestTermVectors` (`lucene/core/src/test`)
//! extends `LuceneTestCase`, not this suite (it only reuses
//! `RandomTokenStream`), so it binds nothing here. The own test methods of
//! the core classes (`testSkipRedundantPrefetches`, `testNoOrds`,
//! `testChunkCleanup`, sprint 9) are declared on their bindings.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene90::term_vectors::Lucene90CompressingTermVectorsReader;
use rucene::codecs::Codec;
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{CodecReader, IndexReader, LeafReader, SeekStatus};
use rucene::store::Directory;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::codecs::compressing::dummy::DummyCompressingCodec;
use rucene_test_framework::codecs::compressing::CompressingCodec;
use rucene_test_framework::index::base_index_file_format_test_case::rethrow;
use rucene_test_framework::index::base_index_file_format_test_case::{
    format_test, tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_term_vectors_format_test_case::{
    self, BaseTermVectorsFormatTestCase,
};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

mod counting_prefetch;
use counting_prefetch::CountingPrefetchDirectory;

/// How a term vectors suite answers `getCodec()`.
enum CodecOf {
    /// A codec built once (a field initializer).
    Field(Arc<dyn Codec>),
    /// A codec built on every call.
    Call(fn() -> Arc<dyn Codec>),
}

/// A term vectors suite: `getCodec()` as `codec` says, and
/// `getReadPastLastPositionExceptionClass()` returning `read_past`.
struct TermVectorsSuite {
    codec: CodecOf,
    read_past: &'static str,
}

impl BaseIndexFileFormatTestCase for TermVectorsSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        match &self.codec {
            CodecOf::Field(codec) => Arc::clone(codec),
            CodecOf::Call(codec) => codec(),
        }
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.term_vectors_add_random_fields(doc);
    }

    fn as_term_vectors(&self) -> Option<&dyn BaseTermVectorsFormatTestCase> {
        Some(self)
    }
}

impl BaseTermVectorsFormatTestCase for TermVectorsSuite {
    fn get_read_past_last_position_exception_class(&self) -> &'static str {
        self.read_past
    }
}

/// The class `name`, extending `BaseTermVectorsFormatTestCase`, whose
/// instances `suite` builds and whose own test methods are `methods`.
fn suite_class(name: &str, suite: fn() -> TermVectorsSuite, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&base_term_vectors_format_test_case::class())
        .constructor(Constructor::no_args(move || {
            FormatTestInstance::new(Arc::new(suite()))
        }));
    for method in methods {
        builder = builder.method(method);
    }
    builder.build()
}

/// `TestLucene90TermVectorsFormat.getCodec()`: `TestUtil.getDefaultCodec()`.
fn lucene90_suite() -> TermVectorsSuite {
    TermVectorsSuite {
        codec: CodecOf::Call(TestUtil::get_default_codec),
        read_past: "java.lang.IllegalStateException",
    }
}

/// `TestLucene90TermVectorsFormat`.
fn lucene90_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.TestLucene90TermVectorsFormat",
        lucene90_suite,
        vec![format_test("testSkipRedundantPrefetches", |_| {
            test_skip_redundant_prefetches()
        })],
    )
}

/// `new FieldType(TextField.TYPE_NOT_STORED)` with `setStoreTermVectors(true)`.
fn term_vectors_type() -> Result<FieldType> {
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_store_term_vectors(true)?;
    Ok(ft)
}

/// Port of `TestLucene90TermVectorsFormat.testSkipRedundantPrefetches`.
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
        let ft = term_vectors_type()?;
        for i in 0..100 {
            let mut doc = Document::new();
            doc.add(Box::new(Field::new("content", i.to_string(), ft.clone())?));
            w.add_document(&doc)?;
        }
        w.force_merge(1)?;
        w.close()?;
    }

    {
        let reader = directory_reader::open(Arc::clone(&dir))?;
        let mut term_vectors = reader.term_vectors()?;
        counter.store(0, Ordering::SeqCst);
        Assert::assert_equals(0, counter.load(Ordering::SeqCst));
        term_vectors.prefetch(0)?;
        Assert::assert_equals(1, counter.load(Ordering::SeqCst));
        term_vectors.prefetch(1)?;
        // This format has 2 docs per block, so the second prefetch is skipped
        Assert::assert_equals(1, counter.load(Ordering::SeqCst));
        term_vectors.prefetch(15)?;
        Assert::assert_equals(2, counter.load(Ordering::SeqCst));
        term_vectors.prefetch(14)?;
        // 14 is in the same block as 15, so the prefetch was skipped
        Assert::assert_equals(2, counter.load(Ordering::SeqCst));
        // Already prefetched in the past, so skipped again
        term_vectors.prefetch(1)?;
        Assert::assert_equals(2, counter.load(Ordering::SeqCst));
        drop(term_vectors);
        reader.close()?;
    }
    drop(dir);
    orig_dir.close()
}

/// `TestCompressingTermVectorsFormat`.
fn compressing_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.compressing.TestCompressingTermVectorsFormat",
        || TermVectorsSuite {
            codec: CodecOf::Call(|| {
                let codec = if *TEST_NIGHTLY {
                    CompressingCodec::random_instance(&*LuceneTestCase::random())
                } else {
                    CompressingCodec::reasonable_instance(&*LuceneTestCase::random())
                };
                Arc::new(codec.unwrap_or_else(|e| rethrow(e)))
            }),
            read_past: "java.lang.IllegalStateException",
        },
        vec![
            format_test("testNoOrds", |_| test_no_ords()),
            format_test("testChunkCleanup", |_| test_chunk_cleanup()),
        ],
    )
}

// https://issues.apache.org/jira/browse/LUCENE-5156
/// Port of `TestCompressingTermVectorsFormat.testNoOrds`.
fn test_no_ords() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    let mut doc = Document::new();
    let ft = term_vectors_type()?;
    doc.add(Box::new(Field::new(
        "foo",
        "this is a test".to_string(),
        ft,
    )?));
    iw.add_document(&doc)?;
    let ir = LuceneTestCase::get_only_leaf_reader(iw.get_reader()? as Arc<dyn IndexReader>)?;
    let terms = ir.term_vectors()?.get_field(0, "foo")?;
    Assert::assert_true(terms.is_some()); // assertNotNull(terms)
    let terms = terms.expect("assertNotNull(terms)");
    let mut terms_enum = terms.iterator()?;
    Assert::assert_true(
        SeekStatus::FOUND == terms_enum.seek_ceil(&BytesRef::from_char_sequence("this"))?,
    );

    expect_throws("java.lang.UnsupportedOperationException", || {
        terms_enum.ord()
    });
    expect_throws("java.lang.UnsupportedOperationException", || {
        terms_enum.seek_ord(0)
    });

    Arc::clone(&ir).as_index_reader().close()?;
    iw.close()?;
    dir.close()
}

/// `(Lucene90CompressingTermVectorsReader) ((CodecReader) reader).getTermVectorsReader()`:
/// the casts' `ClassCastException`, or the `NullPointerException` of the
/// next call on a `null` reader.
fn with_compressing_term_vectors_reader<T>(
    reader: &Arc<dyn LeafReader>,
    body: impl FnOnce(&Lucene90CompressingTermVectorsReader) -> T,
) -> Result<T> {
    let sr = Arc::clone(reader).as_codec_reader().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.ClassCastException",
            "the leaf reader is not a CodecReader",
        )
        .throw()
    });
    let vectors_reader = CodecReader::get_term_vectors_reader(&*sr)?.unwrap_or_else(|| {
        Throwable::new(
            "java.lang.NullPointerException",
            "getTermVectorsReader() is null",
        )
        .throw()
    });
    let compressing = vectors_reader
        .as_any()
        .and_then(|any| any.downcast_ref::<Lucene90CompressingTermVectorsReader>())
        .unwrap_or_else(|| {
            Throwable::new(
                "java.lang.ClassCastException",
                "the term vectors reader is not a Lucene90CompressingTermVectorsReader",
            )
            .throw()
        });
    Ok(body(compressing))
}

/// writes some tiny segments with incomplete compressed blocks, and ensures merge recompresses
/// them.
///
/// Port of `TestCompressingTermVectorsFormat.testChunkCleanup`.
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
        let ft = term_vectors_type()?;
        doc.add(Box::new(Field::new(
            "text",
            "not very long at all".to_string(),
            ft,
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
            with_compressing_term_vectors_reader(&leaf.leaf_reader(), |reader| {
                Assert::assert_true(reader.num_dirty_docs() > 0);
                Assert::assert_equals(1, reader.num_dirty_chunks());
            })?;
        }
    }
    iw.get_config()
        .set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    iw.force_merge(1)?;
    // add one more doc and merge again
    let mut doc = Document::new();
    let ft = term_vectors_type()?;
    doc.add(Box::new(Field::new(
        "text",
        "not very long at all".to_string(),
        ft,
    )?));
    iw.add_document(&doc)?;
    iw.force_merge(1)?;
    let ir2 = directory_reader::open_if_changed(Arc::clone(&ir))?;
    Assert::assert_true(ir2.is_some()); // assertNotNull(ir2)
    let ir2 = ir2.expect("assertNotNull(ir2)");
    ir.close()?;
    ir = ir2;
    let sr = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as Arc<dyn IndexReader>)?;
    with_compressing_term_vectors_reader(&sr, |reader| {
        // at most 2: the 5 chunks from 5 doc segment will be collapsed into a single chunk
        Assert::assert_true(reader.num_dirty_chunks() <= 2);
    })?;
    ir.close()?;
    iw.close()?;
    dir.close()
}

/// `TestAssertingTermVectorsFormat`: `private final Codec codec = new
/// AssertingCodec()`; reading past the last position throws
/// `AssertionError`.
fn asserting_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.tests.codecs.asserting.TestAssertingTermVectorsFormat",
        || TermVectorsSuite {
            codec: CodecOf::Field(Arc::new(AssertingCodec::new())),
            read_past: "java.lang.AssertionError",
        },
        Vec::new(),
    )
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestTermVectorsSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestTermVectorsSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestTermVectorsSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90TermVectorsFormat`, recording the next `random().nextLong()` after
/// each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestTermVectorsSuiteDraws")
        .extends(&base_term_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(lucene90_suite()))
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

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// Seeded-draw parity with Lucene (rmp #352): every row of
/// `base_term_vectors_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_term_vectors_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_term_vectors_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene90_term_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}

#[test]
fn test_compressing_term_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(compressing_class);
}

#[test]
fn test_asserting_term_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(asserting_class);
}
