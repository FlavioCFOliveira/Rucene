//! Port of `lucene/core/src/test/org/apache/lucene/codecs/perfield/TestPerFieldPostingsFormat2.java`.
//!
//! TODO: would be better in this test to pull termsenums and instanceof or something?
//! this way we can verify PFPF is doing the right thing.
//! for now we do termqueries.
//!
//! Java's `AssertingCodec` subclasses (`MockCodec` and the anonymous ones)
//! override `getPostingsFormatForField`; Rucene's `AssertingCodec` takes that
//! choice as a function (`AssertingCodec::with_formats_for_field`).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Test;
use rucene::codecs::{
    Codec, FieldsConsumer, FieldsProducer, MergeState, NormsProducer, PostingsFormat,
    SegmentReadState, SegmentWriteState,
};
use rucene::document::{Document, Field, FieldType, IntPoint, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{LogMergePolicy, MergePolicy};
use rucene::index::{Fields, Term};
use rucene::search::term_query::TermQuery;
use rucene::store::Directory;
use rucene_codecs::memory::DirectPostingsFormat;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::asserting::asserting_codec::{
    AssertingCodec, PostingsFormatForField,
};
use rucene_test_framework::codecs::asserting::asserting_postings_format::AssertingPostingsFormat;
use rucene_test_framework::codecs::blockterms::lucene_var_gap_fixed_interval::LuceneVarGapFixedInterval;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, VERBOSE};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, random, test};

/// `TestPerFieldPostingsFormat2`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.codecs.perfield.TestPerFieldPostingsFormat2",
        vec![
            test(
                "testMergeUnusedPerFieldCodec",
                test_merge_unused_per_field_codec,
            )
            .annotate(Test::new()),
            test("testChangeCodecAndMerge", test_change_codec_and_merge).annotate(Test::new()),
            test("testStressPerFieldCodec", test_stress_per_field_codec).annotate(Test::new()),
            test(
                "testSameCodecDifferentInstance",
                test_same_codec_different_instance,
            ),
            test(
                "testSameCodecDifferentParams",
                test_same_codec_different_params,
            ),
            test(
                "testMergeCalledOnTwoFormats",
                test_merge_called_on_two_formats,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// Port of `TestPerFieldPostingsFormat2.newWriter`.
fn new_writer(dir: Arc<dyn Directory>, conf: IndexWriterConfig) -> Result<Arc<IndexWriter>> {
    let log_byte_size_merge_policy = LogMergePolicy::by_doc_count();
    log_byte_size_merge_policy.set_no_cfs_ratio(0.0)?; // make sure we use plain
                                                       // files
    conf.set_merge_policy(Some(Arc::new(log_byte_size_merge_policy)))?;

    let writer = IndexWriter::new(dir, conf)?;
    Ok(writer)
}

/// `doc.add(newTextField("content", content, Field.Store.NO))`, `numDocs`
/// times.
fn add_content_docs(writer: &IndexWriter, num_docs: i32, content: &str) -> Result<()> {
    for _ in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            content,
            Store::NO,
        )));
        writer.add_document(&doc)?;
    }
    Ok(())
}

/// Port of `TestPerFieldPostingsFormat2.addDocs`.
fn add_docs(writer: &IndexWriter, num_docs: i32) -> Result<()> {
    add_content_docs(writer, num_docs, "aaa")
}

/// Port of `TestPerFieldPostingsFormat2.addDocs2`.
fn add_docs2(writer: &IndexWriter, num_docs: i32) -> Result<()> {
    add_content_docs(writer, num_docs, "bbb")
}

/// Port of `TestPerFieldPostingsFormat2.addDocs3`.
fn add_docs3(writer: &IndexWriter, num_docs: i32) -> Result<()> {
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            "ccc",
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    Ok(())
}

/// Test that heterogeneous index segments are merge successfully
///
/// Port of `TestPerFieldPostingsFormat2.testMergeUnusedPerFieldCodec`.
fn test_merge_unused_per_field_codec() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwconf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    iwconf.set_open_mode(OpenMode::CREATE)?;
    iwconf.set_codec(Arc::new(mock_codec()))?;
    let writer = new_writer(dir.as_directory(), iwconf)?;
    add_docs(&writer, 10)?;
    writer.commit()?;
    add_docs3(&writer, 10)?;
    writer.commit()?;
    add_docs2(&writer, 10)?;
    writer.commit()?;
    assert_equals(&30, &writer.get_doc_stats()?.max_doc);
    TestUtil::check_index(dir.as_directory())?;
    writer.force_merge(1)?;
    assert_equals(&30, &writer.get_doc_stats()?.max_doc);
    writer.close()?;
    dir.close()
}

/// Test that heterogeneous index segments are merged sucessfully
///
/// TODO: not sure this test is that great, we should probably peek inside PerFieldPostingsFormat
/// or something?!
///
/// Port of `TestPerFieldPostingsFormat2.testChangeCodecAndMerge`.
fn test_change_codec_and_merge() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let dir = directory.as_directory();
    if *VERBOSE {
        println!("TEST: make new index");
    }
    let iwconf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    iwconf.set_open_mode(OpenMode::CREATE)?;
    iwconf.set_codec(Arc::new(mock_codec()))?;
    iwconf.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    // ((LogMergePolicy) iwconf.getMergePolicy()).setMergeFactor(10);
    let codec = iwconf.codec();
    let writer = new_writer(Arc::clone(&dir), iwconf)?;

    add_docs(&writer, 10)?;
    writer.commit()?;
    assert_query(Term::from_text("content", "aaa"), &dir, 10)?;
    if *VERBOSE {
        println!("TEST: addDocs3");
    }
    add_docs3(&writer, 10)?;
    writer.commit()?;
    writer.close()?;

    assert_query(Term::from_text("content", "ccc"), &dir, 10)?;
    assert_query(Term::from_text("content", "aaa"), &dir, 10)?;

    let iwconf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    iwconf.set_open_mode(OpenMode::APPEND)?;
    iwconf.set_codec(codec)?;
    // ((LogMergePolicy) iwconf.getMergePolicy()).setNoCFSRatio(0.0);
    // ((LogMergePolicy) iwconf.getMergePolicy()).setMergeFactor(10);
    iwconf.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;

    iwconf.set_codec(Arc::new(mock_codec()))?; // uses standard for field content
    let writer = new_writer(Arc::clone(&dir), iwconf)?;
    // swap in new codec for currently written segments
    if *VERBOSE {
        println!("TEST: add docs w/ Standard codec for content field");
    }
    add_docs2(&writer, 10)?;
    writer.commit()?;
    assert_equals(&30, &writer.get_doc_stats()?.max_doc);
    assert_query(Term::from_text("content", "bbb"), &dir, 10)?;
    assert_query(Term::from_text("content", "ccc"), &dir, 10)?; // //
    assert_query(Term::from_text("content", "aaa"), &dir, 10)?;

    if *VERBOSE {
        println!("TEST: add more docs w/ new codec");
    }
    add_docs2(&writer, 10)?;
    writer.commit()?;
    assert_query(Term::from_text("content", "ccc"), &dir, 10)?;
    assert_query(Term::from_text("content", "bbb"), &dir, 20)?;
    assert_query(Term::from_text("content", "aaa"), &dir, 10)?;
    assert_equals(&40, &writer.get_doc_stats()?.max_doc);

    if *VERBOSE {
        println!("TEST: now optimize");
    }
    writer.force_merge(1)?;
    assert_equals(&40, &writer.get_doc_stats()?.max_doc);
    writer.close()?;
    assert_query(Term::from_text("content", "ccc"), &dir, 10)?;
    assert_query(Term::from_text("content", "bbb"), &dir, 20)?;
    assert_query(Term::from_text("content", "aaa"), &dir, 10)?;

    drop(dir);
    directory.close()
}

/// Port of `TestPerFieldPostingsFormat2.assertQuery`.
fn assert_query(t: Term, dir: &Arc<dyn Directory>, num: i32) -> Result<()> {
    if *VERBOSE {
        println!("\nTEST: assertQuery {t}");
    }
    let reader = directory_reader::open(Arc::clone(dir))?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let search = searcher.search(Arc::new(TermQuery::new(t)), num + 10)?;
    assert_equals(&(num as i64), &search.total_hits.value());
    reader.close()
}

/// `TestPerFieldPostingsFormat2.MockCodec extends AssertingCodec`: `id` gets
/// a `DirectPostingsFormat`, every other field
/// `TestUtil.getDefaultPostingsFormat()`.
fn mock_codec() -> AssertingCodec {
    let lucene_default: Arc<dyn PostingsFormat> = TestUtil::get_default_postings_format();
    let direct: Arc<dyn PostingsFormat> = Arc::new(DirectPostingsFormat::new());
    let for_field: PostingsFormatForField = Arc::new(move |field: &str| {
        if field == "id" {
            Arc::clone(&direct)
        } else {
            Arc::clone(&lucene_default)
        }
    });
    AssertingCodec::with_formats_for_field(Some(for_field), None, None)
}

/// Test per field codec support - adding fields with random codecs
///
/// Port of `TestPerFieldPostingsFormat2.testStressPerFieldCodec`.
fn test_stress_per_field_codec() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory_with(&*random());
    let dir = directory.as_directory();
    let docs_per_round = 97;
    let num_rounds = LuceneTestCase::at_least(1);
    for i in 0..num_rounds {
        let num = TestUtil::next_int(&*random(), 30, 60);
        let config = LuceneTestCase::new_index_writer_config_with(
            &*random(),
            Some(Arc::new(MockAnalyzer::new(&*random()))),
        );
        config.set_open_mode(OpenMode::CREATE_OR_APPEND)?;
        let writer = new_writer(Arc::clone(&dir), config)?;
        for _ in 0..docs_per_round {
            let mut doc = Document::new();
            for k in 0..num {
                let custom_type = FieldType::new_from(TextField::type_not_stored());
                custom_type.set_tokenized(random().next_boolean())?;
                custom_type.set_omit_norms(random().next_boolean())?;
                let field = LuceneTestCase::new_field(
                    &k.to_string(),
                    &TestUtil::random_realistic_unicode_string_max(&*random(), 128),
                    &custom_type,
                );
                doc.add(Box::new(field));
            }
            writer.add_document(&doc)?;
        }
        if random().next_boolean() {
            writer.force_merge(1)?;
        }
        writer.commit()?;
        assert_equals(
            &((i + 1) * docs_per_round),
            &writer.get_doc_stats()?.max_doc,
        );
        writer.close()?;
    }
    drop(dir);
    directory.close()
}

/// `super.getPostingsFormatForField(field)` of an `AssertingCodec`
/// subclass: the codec's own `Asserting` postings format.
fn asserting_default() -> Arc<dyn PostingsFormat> {
    Arc::new(AssertingPostingsFormat::new())
}

/// Port of `TestPerFieldPostingsFormat2.testSameCodecDifferentInstance`.
fn test_same_codec_different_instance() -> Result<()> {
    let super_format = asserting_default();
    let for_field: PostingsFormatForField = Arc::new(move |field: &str| {
        if "id" == field {
            Arc::new(DirectPostingsFormat::new())
        } else if "date" == field {
            Arc::new(DirectPostingsFormat::new())
        } else {
            Arc::clone(&super_format)
        }
    });
    let codec: Arc<dyn Codec> = Arc::new(AssertingCodec::with_formats_for_field(
        Some(for_field),
        None,
        None,
    ));
    do_test_mixed_postings(codec)
}

/// Port of `TestPerFieldPostingsFormat2.testSameCodecDifferentParams`.
fn test_same_codec_different_params() -> Result<()> {
    let super_format = asserting_default();
    let for_field: PostingsFormatForField = Arc::new(move |field: &str| {
        if "id" == field {
            Arc::new(LuceneVarGapFixedInterval::with_interval(1))
        } else if "date" == field {
            Arc::new(LuceneVarGapFixedInterval::with_interval(2))
        } else {
            Arc::clone(&super_format)
        }
    });
    let codec: Arc<dyn Codec> = Arc::new(AssertingCodec::with_formats_for_field(
        Some(for_field),
        None,
        None,
    ));
    do_test_mixed_postings(codec)
}

/// Port of `TestPerFieldPostingsFormat2.doTestMixedPostings`.
fn do_test_mixed_postings(codec: Arc<dyn Codec>) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_codec(codec)?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    let ft = Arc::new(FieldType::new_from(TextField::type_not_stored()));
    // turn on vectors for the checkindex cross-check
    ft.set_store_term_vectors(true)?;
    ft.set_store_term_vector_offsets(true)?;
    ft.set_store_term_vector_positions(true)?;
    let id_field = Field::new("id", String::new(), Arc::clone(&ft))?;
    let date_field = Field::new("date", String::new(), Arc::clone(&ft))?;
    doc.add(Box::new(id_field));
    doc.add(Box::new(date_field));
    for _ in 0..100 {
        field_mut(&mut doc, "id").set_string_value(random().next_int_bound(50).to_string())?;
        field_mut(&mut doc, "date").set_string_value(random().next_int_bound(100).to_string())?;
        iw.add_document(&doc)?;
    }
    iw.close()?;
    dir.close() // checkindex
}

/// The `Field` named `name` that the Java test keeps after `doc.add`.
fn field_mut<'a>(doc: &'a mut Document, name: &str) -> &'a Field {
    doc.get_field_mut_as::<Field>(name)
        .expect("INVARIANT: the document holds the Field")
}

/// Port of `TestPerFieldPostingsFormat2.testMergeCalledOnTwoFormats`.
fn test_merge_called_on_two_formats() -> Result<()> {
    let pf1 = Arc::new(MergeRecordingPostingsFormatWrapper::new(
        TestUtil::get_default_postings_format(),
    ));
    let pf2 = Arc::new(MergeRecordingPostingsFormatWrapper::new(
        TestUtil::get_default_postings_format(),
    ));

    let iwc = IndexWriterConfig::new();
    let super_format = asserting_default();
    let (f1, f2) = (
        Arc::clone(&pf1) as Arc<dyn PostingsFormat>,
        Arc::clone(&pf2) as Arc<dyn PostingsFormat>,
    );
    let for_field: PostingsFormatForField = Arc::new(move |field: &str| match field {
        "f1" | "f2" => Arc::clone(&f1),
        "f3" | "f4" => Arc::clone(&f2),
        _ => Arc::clone(&super_format),
    });
    iwc.set_codec(Arc::new(AssertingCodec::with_formats_for_field(
        Some(for_field),
        None,
        None,
    )))?;

    let mut directory = LuceneTestCase::new_directory();

    let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("f1", "val1".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("f2", "val2".into(), Store::YES)?));
    doc.add(Box::new(IntPoint::new("f3", &[3])?)); // Points are not indexed as postings and should not appear in the merge fields
    doc.add(Box::new(StringField::new("f4", "val4".into(), Store::NO)?));
    iwriter.add_document(&doc)?;
    iwriter.commit()?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("f1", "val5".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("f2", "val6".into(), Store::YES)?));
    doc.add(Box::new(IntPoint::new("f3", &[7])?));
    doc.add(Box::new(StringField::new("f4", "val8".into(), Store::NO)?));
    iwriter.add_document(&doc)?;
    iwriter.commit()?;

    iwriter.force_merge_with(1, true)?;
    iwriter.close()?;

    let (calls1, names1) = pf1.recorded();
    let (calls2, names2) = pf2.recorded();
    assert_equals(&1, &calls1);
    assert_equals(
        &HashSet::from(["f1".to_string(), "f2".to_string()]),
        &names1.into_iter().collect::<HashSet<String>>(),
    );
    assert_equals(&1, &calls2);
    assert_equals(&vec!["f4".to_string()], &names2);

    directory.close()
}

/// `fieldNames` and `nbMergeCalls` of a [`MergeRecordingPostingsFormatWrapper`].
#[derive(Debug, Default)]
struct MergeRecord {
    field_names: Vec<String>,
    nb_merge_calls: i32,
}

/// Port of `TestPerFieldPostingsFormat2.MergeRecordingPostingsFormatWrapper`.
#[derive(Debug)]
struct MergeRecordingPostingsFormatWrapper {
    name: String,
    delegate: Arc<dyn PostingsFormat>,
    record: Arc<Mutex<MergeRecord>>,
}

impl MergeRecordingPostingsFormatWrapper {
    /// `MergeRecordingPostingsFormatWrapper(delegate)`: `super(delegate.getName())`.
    fn new(delegate: Arc<dyn PostingsFormat>) -> Self {
        Self {
            name: delegate.name().to_string(),
            delegate,
            record: Arc::new(Mutex::new(MergeRecord::default())),
        }
    }

    /// `nbMergeCalls` and `fieldNames`.
    fn recorded(&self) -> (i32, Vec<String>) {
        let record = self.record.lock().unwrap_or_else(|e| e.into_inner());
        (record.nb_merge_calls, record.field_names.clone())
    }
}

impl PostingsFormat for MergeRecordingPostingsFormatWrapper {
    fn name(&self) -> &str {
        &self.name
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let consumer = self.delegate.fields_consumer(state)?;
        Ok(Box::new(MergeRecordingFieldsConsumer {
            consumer,
            record: Arc::clone(&self.record),
        }))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        self.delegate.fields_producer(state)
    }
}

/// The anonymous `FieldsConsumer` of
/// `MergeRecordingPostingsFormatWrapper.fieldsConsumer`.
struct MergeRecordingFieldsConsumer<'a> {
    consumer: Box<dyn FieldsConsumer + 'a>,
    record: Arc<Mutex<MergeRecord>>,
}

impl FieldsConsumer for MergeRecordingFieldsConsumer<'_> {
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        self.consumer.write(fields, norms)
    }

    fn merge(&mut self, merge_state: &MergeState, norms: Option<&dyn NormsProducer>) -> Result<()> {
        {
            let mut record = self.record.lock().unwrap_or_else(|e| e.into_inner());
            record.nb_merge_calls += 1;
            for fi in merge_state.merge_field_infos.iter() {
                record.field_names.push(fi.name.clone());
            }
        }
        self.consumer.merge(merge_state, norms)
    }

    fn close(&mut self) -> Result<()> {
        self.consumer.close()
    }
}
