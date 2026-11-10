//! The doc values suites (rmp #353, #359): the inherited cases of
//! `BaseIndexFileFormatTestCase`, `LegacyBaseDocValuesFormatTestCase`,
//! `BaseDocValuesFormatTestCase` and, for the classes that extend it,
//! `BaseCompressingDocValuesFormatTestCase`, run under the ported runner and
//! `LuceneTestCase`'s rule chain on the codecs Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormat`
//!   (`lucene/core/src/test`, extends `BaseCompressingDocValuesFormatTestCase`):
//!   `private final Codec codec = TestUtil.getDefaultCodec()`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormatMergeInstance`
//!   (`lucene/core/src/test`): extends `TestLucene90DocValuesFormat`,
//!   overriding `shouldTestMergeInstance()` to return `true`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormatVariableSkipInterval`
//!   (`lucene/core/src/test`, extends `BaseDocValuesFormatTestCase`):
//!   `getCodec()` returns `TestUtil.alwaysDocValuesFormat(new
//!   Lucene90DocValuesFormat(random().nextInt(4, 16)))`;
//! * `org.apache.lucene.codecs.perfield.TestPerFieldDocValuesFormat`
//!   (`lucene/core/src/test`, extends `BaseDocValuesFormatTestCase`):
//!   `setUp()` sets `codec = new RandomCodec(new Random(random().nextLong()),
//!   Collections.emptySet())` before `super.setUp()`, and
//!   `codecAcceptsHugeBinaryValues(field)` returns
//!   `TestUtil.fieldSupportsHugeBinaryDocValues(field)`;
//! * `org.apache.lucene.tests.codecs.asserting.TestAssertingDocValuesFormat`
//!   (`lucene/test-framework/src/test`, ported in full): `AssertingCodec`.
//!
//! The own test methods of the core classes (sprint 9) are declared on their
//! bindings; `TestLucene90DocValuesFormatMergeInstance` inherits those of
//! `TestLucene90DocValuesFormat`.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene90::doc_values::Lucene90DocValuesFormat;
use rucene::codecs::{
    Codec, DocValuesConsumer, DocValuesFormat, DocValuesProducer, MergeState, SegmentReadState,
    SegmentWriteState,
};
use rucene::document::{BinaryDocValuesField, Document, NumericDocValuesField, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::field_infos::FieldInfo;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::asserting::asserting_codec::{
    AssertingCodec, DocValuesFormatForField,
};
use rucene_test_framework::index::base_compressing_doc_values_format_test_case::{
    self, BaseCompressingDocValuesFormatTestCase,
};
use rucene_test_framework::index::base_doc_values_format_test_case::{
    self, BaseDocValuesFormatTestCase,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    format_test, set_up, tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::legacy_base_doc_values_format_test_case::LegacyBaseDocValuesFormatTestCase;
use rucene_test_framework::index::RandomCodec;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

mod lucene90_doc_values;

/// A subclass of `BaseCompressingDocValuesFormatTestCase` whose `codec`
/// field holds the codec `getCodec()` returns, overriding
/// `shouldTestMergeInstance()` when `merge_instance` is set.
struct CompressingDocValuesSuite {
    codec: Arc<dyn Codec>,
    merge_instance: bool,
}

impl BaseIndexFileFormatTestCase for CompressingDocValuesSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn should_test_merge_instance(&self) -> bool {
        self.merge_instance
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.doc_values_add_random_fields(doc);
    }

    fn as_doc_values(&self) -> Option<&dyn LegacyBaseDocValuesFormatTestCase> {
        Some(self)
    }
}

impl LegacyBaseDocValuesFormatTestCase for CompressingDocValuesSuite {
    fn as_base_doc_values(&self) -> Option<&dyn BaseDocValuesFormatTestCase> {
        Some(self)
    }

    fn as_compressing_doc_values(&self) -> Option<&dyn BaseCompressingDocValuesFormatTestCase> {
        Some(self)
    }
}

impl BaseDocValuesFormatTestCase for CompressingDocValuesSuite {}

impl BaseCompressingDocValuesFormatTestCase for CompressingDocValuesSuite {}

/// `TestLucene90DocValuesFormat`.
fn lucene90_class() -> Arc<Class> {
    let mut builder = Class::builder(lucene90_doc_values::LUCENE90_CLASS)
        .extends(&base_compressing_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompressingDocValuesSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: false,
            }))
        }));
    for method in lucene90_doc_values::lucene90_methods() {
        builder = builder.method(method);
    }
    builder.build()
}

/// `TestLucene90DocValuesFormatMergeInstance`: `extends
/// TestLucene90DocValuesFormat` and overrides `shouldTestMergeInstance()` to
/// return `true`.
fn merge_instance_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormatMergeInstance")
        .extends(&lucene90_class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompressingDocValuesSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: true,
            }))
        }))
        .build()
}

/// How a `BaseDocValuesFormatTestCase` subclass answers `getCodec()`.
enum CodecOf {
    /// A codec built once (a field initializer).
    Field(Arc<dyn Codec>),
    /// A codec built on every call.
    Call(fn() -> Arc<dyn Codec>),
    /// The codec `setUp()` sets (`TestPerFieldDocValuesFormat`).
    SetUp(Mutex<Option<Arc<dyn Codec>>>),
}

/// A subclass of `BaseDocValuesFormatTestCase`: `getCodec()` as `codec`
/// says; `huge_binary_by_field` overrides `codecAcceptsHugeBinaryValues`
/// with `TestUtil.fieldSupportsHugeBinaryDocValues(field)`.
struct DocValuesSuite {
    codec: CodecOf,
    huge_binary_by_field: bool,
}

impl BaseIndexFileFormatTestCase for DocValuesSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        match &self.codec {
            CodecOf::Field(codec) => Arc::clone(codec),
            CodecOf::Call(codec) => codec(),
            CodecOf::SetUp(codec) => codec
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .unwrap_or_else(|| {
                    randomizedtesting::jdk::exception::throw_without_message(
                        "java.lang.NullPointerException",
                    )
                }),
        }
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.doc_values_add_random_fields(doc);
    }

    fn as_doc_values(&self) -> Option<&dyn LegacyBaseDocValuesFormatTestCase> {
        Some(self)
    }
}

impl LegacyBaseDocValuesFormatTestCase for DocValuesSuite {
    fn as_base_doc_values(&self) -> Option<&dyn BaseDocValuesFormatTestCase> {
        Some(self)
    }

    fn codec_accepts_huge_binary_values(&self, field: &str) -> bool {
        if self.huge_binary_by_field {
            TestUtil::field_supports_huge_binary_doc_values(field)
        } else {
            true
        }
    }
}

impl BaseDocValuesFormatTestCase for DocValuesSuite {}

/// `TestLucene90DocValuesFormatVariableSkipInterval`.
fn variable_skip_interval_class() -> Arc<Class> {
    let mut builder = Class::builder(lucene90_doc_values::VARIABLE_SKIP_INTERVAL_CLASS)
        .extends(&base_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(DocValuesSuite {
                codec: CodecOf::Call(|| {
                    TestUtil::always_doc_values_format(Arc::new(
                        Lucene90DocValuesFormat::with_interval(
                            LuceneTestCase::random().next_int_range(4, 16),
                        ),
                    ))
                }),
                huge_binary_by_field: false,
            }))
        }));
    for method in lucene90_doc_values::variable_skip_interval_methods() {
        builder = builder.method(method);
    }
    builder.build()
}

/// `TestPerFieldDocValuesFormat`.
fn per_field_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.perfield.TestPerFieldDocValuesFormat")
        .extends(&base_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(DocValuesSuite {
                codec: CodecOf::SetUp(Mutex::new(None)),
                huge_binary_by_field: true,
            }))
        }))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut FormatTestInstance| {
                let suite = inst.subclass::<DocValuesSuite>();
                if let CodecOf::SetUp(codec) = &suite.codec {
                    let random = JdkRandom::new(LuceneTestCase::random().next_long());
                    *codec.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(Arc::new(RandomCodec::new_avoiding(&random, HashSet::new())));
                }
                set_up(inst);
            },
        ))
        .method(format_test("testTwoFieldsTwoFormats", |_| {
            test_two_fields_two_formats()
        }))
        .method(format_test("testMergeCalledOnTwoFormats", |_| {
            test_merge_called_on_two_formats()
        }))
        .method(format_test("testDocValuesMergeWithIndexedFields", |_| {
            test_doc_values_merge_with_indexed_fields()
        }))
        .build()
}

/// `new AssertingCodec() { getDocValuesFormatForField(field) { ... } }`.
fn asserting_codec_with_doc_values(for_field: DocValuesFormatForField) -> Arc<dyn Codec> {
    Arc::new(AssertingCodec::with_formats_for_field(
        None,
        Some(for_field),
        None,
    ))
}

/// `AssertingCodec.getDocValuesFormatForField(field)`, the `super` call of
/// the anonymous subclasses: the codec's own `Asserting` format.
fn asserting_default_doc_values_format() -> Arc<dyn DocValuesFormat> {
    Arc::new(rucene_test_framework::codecs::asserting::asserting_doc_values_format::AssertingDocValuesFormat::new())
}

// just a simple trivial test
// TODO: we should come up with a test that somehow checks that segment suffix
// is respected by all codec apis (not just docvalues and postings)
/// Port of `TestPerFieldDocValuesFormat.testTwoFieldsTwoFormats`.
fn test_two_fields_two_formats() -> Result<()> {
    let analyzer = Arc::new(MockAnalyzer::new(&*LuceneTestCase::random()));

    let mut directory = LuceneTestCase::new_directory();
    // we don't use RandomIndexWriter because it might add more docvalues than we expect !!!!1
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
    let fast = TestUtil::get_default_doc_values_format();
    let slow = rucene::codecs::doc_values::doc_values_for_name("Asserting").unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "An SPI class of type org.apache.lucene.codecs.DocValuesFormat with name 'Asserting' does not exist.",
        )
        .throw()
    });
    iwc.set_codec(asserting_codec_with_doc_values(Arc::new(
        move |field: &str| {
            if "dv1" == field {
                Arc::clone(&fast)
            } else {
                Arc::clone(&slow)
            }
        },
    )))?;
    let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;
    let mut doc = Document::new();
    let long_term = concat!(
        "longtermlongtermlongtermlongtermlongtermlongtermlongtermlongterm",
        "longtermlongtermlongtermlongtermlongtermlongtermlong",
        "termlongtermlongtermlongterm"
    );
    let text = format!("This is the text to be indexed. {long_term}");
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "fieldname",
        &text,
        Store::YES,
    )));
    doc.add(Box::new(NumericDocValuesField::new("dv1", 5)));
    doc.add(Box::new(BinaryDocValuesField::new(
        "dv2",
        BytesRef::from_char_sequence("hello world"),
    )));
    iwriter.add_document(&doc)?;
    iwriter.close()?;

    // Now search the index:
    let ireader = directory_reader::open(directory.as_directory())?; // read-only=true
    let isearcher = LuceneTestCase::new_searcher(Arc::clone(&ireader));

    Assert::assert_equals(
        1,
        isearcher.count(Arc::new(TermQuery::new(Term::new(
            "fieldname",
            BytesRef::from_char_sequence(long_term),
        ))))?,
    );
    let query = Arc::new(TermQuery::new(Term::new(
        "fieldname",
        BytesRef::from_char_sequence("text"),
    )));
    let hits = isearcher.search(query, 1)?;
    Assert::assert_equals(1, hits.total_hits.value());
    let stored_fields = isearcher.stored_fields()?;
    // Iterate through the results:
    for score_doc in &hits.score_docs {
        let hit_doc_id = score_doc.doc;
        let hit_doc = stored_fields.document(hit_doc_id)?;
        Assert::assert_equals_str(None, Some(&text), hit_doc.get("fieldname").as_deref());
        debug_assert!(Arc::clone(&ireader).leaves()?.len() == 1);
        let leaf = Arc::clone(&ireader).leaves()?[0].leaf_reader();
        let mut dv = leaf.get_numeric_doc_values("dv1")?.unwrap_or_else(|| {
            Throwable::new(
                "java.lang.NullPointerException",
                "getNumericDocValues(\"dv1\")",
            )
            .throw()
        });
        Assert::assert_equals(hit_doc_id, dv.advance(hit_doc_id)?);
        Assert::assert_equals(5, dv.long_value()?);

        let mut dv2 = leaf.get_binary_doc_values("dv2")?.unwrap_or_else(|| {
            Throwable::new(
                "java.lang.NullPointerException",
                "getBinaryDocValues(\"dv2\")",
            )
            .throw()
        });
        Assert::assert_equals(hit_doc_id, dv2.advance(hit_doc_id)?);
        let term = dv2.binary_value()?;
        Assert::assert_true(BytesRef::from_char_sequence("hello world") == term);
    }

    drop(stored_fields);
    drop(isearcher);
    ireader.close()?;
    directory.close()
}

/// Port of `TestPerFieldDocValuesFormat.testMergeCalledOnTwoFormats`.
fn test_merge_called_on_two_formats() -> Result<()> {
    let dvf1 = Arc::new(MergeRecordingDocValueFormatWrapper::new(
        TestUtil::get_default_doc_values_format(),
    ));
    let dvf2 = Arc::new(MergeRecordingDocValueFormatWrapper::new(
        TestUtil::get_default_doc_values_format(),
    ));

    let mut iwc = IndexWriterConfig::new();
    let (f1, f2) = (
        Arc::clone(&dvf1) as Arc<dyn DocValuesFormat>,
        Arc::clone(&dvf2) as Arc<dyn DocValuesFormat>,
    );
    let super_format = asserting_default_doc_values_format();
    iwc.set_codec(asserting_codec_with_doc_values(Arc::new(
        move |field: &str| match field {
            "dv1" | "dv2" => Arc::clone(&f1),
            "dv3" => Arc::clone(&f2),
            _ => Arc::clone(&super_format),
        },
    )))?;

    let mut directory = LuceneTestCase::new_directory();

    let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("dv1", 5)));
    doc.add(Box::new(NumericDocValuesField::new("dv2", 42)));
    doc.add(Box::new(BinaryDocValuesField::new(
        "dv3",
        BytesRef::from_char_sequence("hello world"),
    )));
    iwriter.add_document(&doc)?;
    iwriter.commit()?;

    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("dv1", 8)));
    doc.add(Box::new(NumericDocValuesField::new("dv2", 45)));
    doc.add(Box::new(BinaryDocValuesField::new(
        "dv3",
        BytesRef::from_char_sequence("goodbye world"),
    )));
    iwriter.add_document(&doc)?;
    iwriter.commit()?;

    iwriter.force_merge_with(1, true)?;
    iwriter.close()?;

    let (calls1, names1) = dvf1.recorded();
    let (calls2, names2) = dvf2.recorded();
    Assert::assert_equals(1, calls1);
    Assert::assert_true(
        HashSet::from(["dv1".to_string(), "dv2".to_string()])
            == names1.into_iter().collect::<HashSet<String>>(),
    );
    Assert::assert_equals(1, calls2);
    Assert::assert_true(vec!["dv3".to_string()] == names2);

    directory.close()
}

/// Port of `TestPerFieldDocValuesFormat.testDocValuesMergeWithIndexedFields`.
fn test_doc_values_merge_with_indexed_fields() -> Result<()> {
    let doc_values_format = Arc::new(MergeRecordingDocValueFormatWrapper::new(
        TestUtil::get_default_doc_values_format(),
    ));

    let mut iwc = IndexWriterConfig::new();
    let format = Arc::clone(&doc_values_format) as Arc<dyn DocValuesFormat>;
    iwc.set_codec(asserting_codec_with_doc_values(Arc::new(
        move |_field: &str| Arc::clone(&format),
    )))?;

    let mut directory = LuceneTestCase::new_directory();

    let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("dv1", 5)));
    doc.add(Box::new(TextField::new(
        "normalField",
        "not a doc value".to_string(),
        Store::NO,
    )?));
    iwriter.add_document(&doc)?;
    iwriter.commit()?;

    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "anotherField",
        "again no doc values here".to_string(),
        Store::NO,
    )?));
    doc.add(Box::new(TextField::new(
        "normalField",
        "my document without doc values".to_string(),
        Store::NO,
    )?));
    iwriter.add_document(&doc)?;
    iwriter.commit()?;

    iwriter.force_merge_with(1, true)?;
    iwriter.close()?;

    // "normalField" and "anotherField" are ignored when merging doc values.
    let (calls, names) = doc_values_format.recorded();
    Assert::assert_equals(1, calls);
    Assert::assert_true(vec!["dv1".to_string()] == names);
    directory.close()
}

/// `nbMergeCalls` and `fieldNames` of a `MergeRecordingDocValueFormatWrapper`.
#[derive(Debug, Default)]
struct MergeRecord {
    nb_merge_calls: i32,
    field_names: Vec<String>,
}

/// `TestPerFieldDocValuesFormat.MergeRecordingDocValueFormatWrapper`
/// (`private static final class … extends DocValuesFormat`).
#[derive(Debug)]
struct MergeRecordingDocValueFormatWrapper {
    name: String,
    delegate: Arc<dyn DocValuesFormat>,
    record: Arc<Mutex<MergeRecord>>,
}

impl MergeRecordingDocValueFormatWrapper {
    /// `MergeRecordingDocValueFormatWrapper(delegate)`: `super(delegate.getName())`.
    fn new(delegate: Arc<dyn DocValuesFormat>) -> Self {
        Self {
            name: delegate.name().to_string(),
            delegate,
            record: Arc::new(Mutex::new(MergeRecord::default())),
        }
    }

    /// `nbMergeCalls` and `fieldNames`.
    fn recorded(&self) -> (i32, Vec<String>) {
        let record = self.record.lock().unwrap_or_else(PoisonError::into_inner);
        (record.nb_merge_calls, record.field_names.clone())
    }
}

impl DocValuesFormat for MergeRecordingDocValueFormatWrapper {
    fn name(&self) -> &str {
        &self.name
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
        let consumer = self.delegate.fields_consumer(state)?;
        Ok(Box::new(MergeRecordingDocValuesConsumer {
            consumer,
            record: Arc::clone(&self.record),
        }))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>> {
        self.delegate.fields_producer(state)
    }
}

/// The anonymous `DocValuesConsumer` of
/// `MergeRecordingDocValueFormatWrapper.fieldsConsumer`.
#[derive(Debug)]
struct MergeRecordingDocValuesConsumer<'a> {
    consumer: Box<dyn DocValuesConsumer + 'a>,
    record: Arc<Mutex<MergeRecord>>,
}

impl DocValuesConsumer for MergeRecordingDocValuesConsumer<'_> {
    fn add_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.consumer.add_numeric_field(field, values)
    }

    fn add_binary_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.consumer.add_binary_field(field, values)
    }

    fn add_sorted_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.consumer.add_sorted_field(field, values)
    }

    fn add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.consumer.add_sorted_numeric_field(field, values)
    }

    fn add_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.consumer.add_sorted_set_field(field, values)
    }

    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        {
            let mut record = self.record.lock().unwrap_or_else(PoisonError::into_inner);
            record.nb_merge_calls += 1;
            for fi in merge_state.merge_field_infos.iter() {
                record.field_names.push(fi.name.clone());
            }
        }
        self.consumer.merge(merge_state)
    }

    fn close(&mut self) -> Result<()> {
        self.consumer.close()
    }
}

/// `TestAssertingDocValuesFormat`: `private final Codec codec = new
/// AssertingCodec()`.
fn asserting_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.codecs.asserting.TestAssertingDocValuesFormat")
        .extends(&base_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(DocValuesSuite {
                codec: CodecOf::Field(Arc::new(AssertingCodec::new())),
                huge_binary_by_field: false,
            }))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestDocValuesSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestDocValuesSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestDocValuesSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90DocValuesFormat` (so the cases of the three doc values
/// suites), recording the next `random().nextLong()` after each test and
/// after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestDocValuesSuiteDraws")
        .extends(&base_compressing_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompressingDocValuesSuite {
                codec: TestUtil::get_default_codec(),
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

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// Seeded-draw parity with Lucene (rmp #353): every row of
/// `base_doc_values_draws.tsv` (`BaseFormatSuitesOracle`) must be equal,
/// run under the JVM system properties the fixture records (they set the
/// `.si` diagnostics, hence segment sizes and `TieredMergePolicy`'s merge
/// order, hence the merged doc order `testSortedSetVariableLengthSingleValuedVsStoredFields`
/// draws over on DEADBEEF).
///
/// One row is left out: the `next` row of `testNumericDocValuesWithSkipperMedium`
/// on C0FFEE. Java itself does not reproduce it: the test's
/// `RandomIndexWriter` config can pick `ConcurrentMergeScheduler`, whose
/// background merge timing changes the segments the reader sees and so the
/// draws (5 Java runs of that seed gave 5 different values, none the
/// fixture's). The test still runs, and its failure rows are still compared.
#[test]
fn test_doc_values_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match_excluding(
        "base_doc_values_draws.tsv",
        draws_class,
        &DRAWS,
        &[("C0FFEE", "testNumericDocValuesWithSkipperMedium")],
    );
}

#[test]
fn test_lucene90_doc_values_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}

#[test]
fn test_lucene90_doc_values_format_merge_instance() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(merge_instance_class);
}

#[test]
fn test_lucene90_doc_values_format_variable_skip_interval() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(variable_skip_interval_class);
}

#[test]
fn test_per_field_doc_values_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(per_field_class);
}

#[test]
fn test_asserting_doc_values_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(asserting_class);
}
