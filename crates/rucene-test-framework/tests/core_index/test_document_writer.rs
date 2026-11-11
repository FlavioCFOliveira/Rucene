//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocumentWriter.java`.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{
    BinaryDocValuesField, Document, FieldType, IntPoint, InvertableType, KnnFloatVectorField,
    NumericDocValuesField, NumericValue, SharedReader, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField, StoredValue,
    StringField, TextField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::POSTINGS_ENUM_ALL;
use rucene::index::segment_reader::SegmentReader;
use rucene::index::terms::Term;
use rucene::index::{
    IndexOptions, IndexReader, IndexableField, IndexableFieldType, LeafReader, SegmentCommitInfo,
    VectorSimilarityFunction,
};
use rucene::search::NO_MORE_DOCS;
use rucene::util::attribute::{AttributeSource, CapturedState};
use rucene::util::{BytesRef, Version};
use rucene_test_framework::analysis::attributes::{char_term_id, payload_id, pos_inc_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::analysis::{components, shared, sink, tokenizer_components};
use rucene_test_framework::index::doc_helper::{self, DocHelper};
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::hamcrest::assert_less_than;
use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, random, IAE,
};

/// `TestDocumentWriter`: its field `dir`, set by `setUp()` and closed by
/// `tearDown()`.
#[derive(Default)]
struct Instance {
    dir: Option<TestDirectory>,
}

impl Instance {
    fn dir(&self) -> Arc<dyn rucene::store::Directory> {
        self.dir
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("dir"))
            .as_directory()
    }
}

/// A test method of the instance.
fn ti(name: &str, body: fn(&Instance) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut Instance| body(inst).unwrap_or_else(|e| rethrow_java(e)))
}

/// `TestDocumentWriter`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestDocumentWriter")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(Instance::default))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut Instance| {
                LuceneTestCase::super_set_up();
                inst.dir = Some(LuceneTestCase::new_directory());
            },
        ))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut Instance| {
                    if let Some(dir) = inst.dir.take() {
                        dir.close().unwrap_or_else(|e| rethrow_java(e));
                    }
                    LuceneTestCase::super_tear_down();
                }),
        )
        .method(ti("testAddDocument", test_add_document))
        .method(ti("testPositionIncrementGap", test_position_increment_gap))
        .method(ti("testTokenReuse", test_token_reuse))
        .method(ti("testPreAnalyzedField", test_pre_analyzed_field))
        .method(ti("testLUCENE_1590", test_lucene_1590))
        .method(ti("testRAMUsageStored", test_ram_usage_stored))
        .method(ti("testRAMUsageIndexed", test_ram_usage_indexed))
        .method(ti("testRAMUsagePoint", test_ram_usage_point))
        .method(ti(
            "testRAMUsageNumericDocValue",
            test_ram_usage_numeric_doc_value,
        ))
        .method(ti(
            "testRAMUsageSortedDocValue",
            test_ram_usage_sorted_doc_value,
        ))
        .method(ti(
            "testRAMUsageBinaryDocValue",
            test_ram_usage_binary_doc_value,
        ))
        .method(ti(
            "testRAMUsageSortedNumericDocValue",
            test_ram_usage_sorted_numeric_doc_value,
        ))
        .method(ti(
            "testRAMUsageSortedSetDocValue",
            test_ram_usage_sorted_set_doc_value,
        ))
        .method(ti("testRAMUsageVector", test_ram_usage_vector))
        .method(ti(
            "testIndexBinaryValueWithoutTokenStream",
            test_index_binary_value_without_token_stream,
        ))
        .build()
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `new SegmentReader(info, Version.LATEST.major, newIOContext(random()))`.
fn open_segment(info: &SegmentCommitInfo) -> Result<Arc<dyn LeafReader>> {
    let context = LuceneTestCase::new_io_context(&*random());
    Ok(Arc::new(SegmentReader::new(
        info.clone(),
        i32::from(Version::LATEST.major),
        &*context,
    )?))
}

/// `writer.addDocument(doc); writer.commit(); SegmentCommitInfo info =
/// writer.newestSegment(); writer.close();`
fn add_commit_newest(writer: &IndexWriter, doc: &Document) -> Result<SegmentCommitInfo> {
    writer.add_document(doc)?;
    writer.commit()?;
    let info = writer
        .newest_segment()?
        .unwrap_or_else(|| crate::support::npe("writer.newestSegment()"));
    writer.close()?;
    Ok(info)
}

/// `reader.close()` on a leaf.
fn close_leaf(reader: &Arc<dyn LeafReader>) -> Result<()> {
    Arc::clone(reader).as_index_reader().close()
}

/// `doc.getFields(name)`, which must hold one field whose `stringValue()`
/// is `text`.
fn assert_single_string_field(doc: &Document, name: &str, text: &str) -> bool {
    let fields = doc.get_fields_by_name(name);
    Assert::assert_true(fields.len() == 1);
    Assert::assert_true(fields[0].string_value().as_deref() == Some(text));
    fields[0].field_type().store_term_vectors()
}

/// Port of `TestDocumentWriter.testAddDocument`.
fn test_add_document(inst: &Instance) -> Result<()> {
    let mut test_doc = Document::new();
    DocHelper::setup_doc(&mut test_doc);
    let writer = IndexWriter::new(inst.dir(), mock_iwc())?;
    let info = add_commit_newest(&writer, &test_doc)?;
    // After adding the document, we should be able to read it back in
    let reader = open_segment(&info)?;
    let doc = reader.stored_fields()?.document(0)?;

    // System.out.println("Document: " + doc);
    Assert::assert_true(assert_single_string_field(
        &doc,
        "textField2",
        doc_helper::FIELD_2_TEXT,
    ));
    Assert::assert_false(assert_single_string_field(
        &doc,
        "textField1",
        doc_helper::FIELD_1_TEXT,
    ));
    assert_single_string_field(&doc, "keyField", doc_helper::KEYWORD_TEXT);
    assert_single_string_field(&doc, doc_helper::NO_NORMS_KEY, doc_helper::NO_NORMS_TEXT);
    assert_single_string_field(&doc, doc_helper::TEXT_FIELD_3_KEY, doc_helper::FIELD_3_TEXT);

    // test that the norms are not present in the segment if
    // omitNorms is true
    for fi in reader.get_field_infos().iter() {
        if fi.get_index_options() != IndexOptions::NONE {
            Assert::assert_true(fi.omits_norms() == reader.get_norm_values(&fi.name)?.is_none());
        }
    }
    close_leaf(&reader)
}

/// The anonymous `Analyzer` of `testPositionIncrementGap`: a whitespace
/// `MockTokenizer` and a position increment gap of 500.
#[derive(Debug)]
struct GapAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for GapAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestDocumentWriter$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
            MockTokenizer::new(Arc::clone(&WHITESPACE), false)?,
        ))))
    }

    fn get_position_increment_gap(&self, _field_name: &str) -> i32 {
        500
    }
}

/// `MultiTerms.getTermPostingsEnum(reader, field, new BytesRef(text))`.
fn term_postings(
    reader: &Arc<dyn LeafReader>,
    field: &str,
    text: &str,
) -> Result<Box<dyn rucene::index::postings_enum::PostingsEnum>> {
    Ok(MultiTerms::get_term_postings_enum_all(
        &Arc::clone(reader).as_index_reader(),
        field,
        &BytesRef::from_char_sequence(text),
    )?
    .unwrap_or_else(|| crate::support::npe("MultiTerms.getTermPostingsEnum")))
}

/// Port of `TestDocumentWriter.testPositionIncrementGap`.
fn test_position_increment_gap(inst: &Instance) -> Result<()> {
    let analyzer = GapAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    };

    let writer = IndexWriter::new(
        inst.dir(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer))),
    )?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "repeated",
        "repeated one",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "repeated",
        "repeated two",
        Store::YES,
    )));

    let info = add_commit_newest(&writer, &doc)?;
    let reader = open_segment(&info)?;

    let mut term_positions = term_postings(&reader, "repeated", "repeated")?;
    Assert::assert_true(term_positions.next_doc()? != NO_MORE_DOCS);
    let freq = term_positions.freq()?;
    assert_equals(&2, &freq);
    assert_equals(&0, &term_positions.next_position()?);
    assert_equals(&502, &term_positions.next_position()?);
    close_leaf(&reader)
}

/// The anonymous `TokenFilter` of `testTokenReuse`, which indexes a
/// "synonym" `b` after every token.
#[derive(Debug)]
struct SynonymFilter {
    first: bool,
    state: Option<CapturedState>,
    term_att: Att,
    payload_att: Att,
    pos_incr_att: Att,
}

impl TokenFilterLogic for SynonymFilter {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if let Some(state) = self.state.take() {
            source.restore_state(&state)?;
            self.payload_att.payload_mut().set_payload(None);
            self.pos_incr_att.pos_inc_mut().set_position_increment(0);
            {
                let mut term = self.term_att.char_term_mut();
                term.set_empty();
                term.append_string(Some("b"));
            }
            return Ok(true);
        }

        let has_next = input.increment_token()?;
        if !has_next {
            return Ok(false);
        }
        let c = self.term_att.char_term().buffer()[0];
        // `Character.isDigit(c)`: the tokens of this test are ASCII.
        if c.is_ascii_digit() {
            self.pos_incr_att
                .pos_inc_mut()
                .set_position_increment(c as i32 - '0' as i32);
        }
        if self.first {
            // set payload on first position only
            self.payload_att
                .payload_mut()
                .set_payload(Some(BytesRef::new(vec![100])));
            self.first = false;
        }

        // index a "synonym" for every token
        self.state = source.capture_state();
        Ok(true)
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.first = true;
        self.state = None;
        Ok(())
    }
}

/// The anonymous `Analyzer` of `testTokenReuse`.
#[derive(Debug)]
struct SynonymAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for SynonymAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestDocumentWriter$2"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
        // The filter shares its input's attributes: Java's field
        // initializers `addAttribute(...)` on the filter add them there.
        let term_att = Att::add(tokenizer.attribute_source_mut(), char_term_id())?;
        let payload_att = Att::add(tokenizer.attribute_source_mut(), payload_id())?;
        let pos_incr_att = Att::add(tokenizer.attribute_source_mut(), pos_inc_id())?;
        let tokenizer = Arc::new(AtomicRefCell::new(tokenizer));
        let filter = TokenFilter::new(
            shared(&tokenizer),
            SynonymFilter {
                first: true,
                state: None,
                term_att,
                payload_att,
                pos_incr_att,
            },
        );
        Ok(components(tokenizer, sink(filter)))
    }
}

/// Port of `TestDocumentWriter.testTokenReuse`.
fn test_token_reuse(inst: &Instance) -> Result<()> {
    let analyzer = SynonymAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    };

    let writer = IndexWriter::new(
        inst.dir(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer))),
    )?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "f1",
        "a 5 a a",
        Store::YES,
    )));

    let info = add_commit_newest(&writer, &doc)?;
    let reader = open_segment(&info)?;

    let mut term_positions = term_postings(&reader, "f1", "a")?;
    Assert::assert_true(term_positions.next_doc()? != NO_MORE_DOCS);
    let freq = term_positions.freq()?;
    assert_equals(&3, &freq);
    assert_equals(&0, &term_positions.next_position()?);
    assert_not_null(&term_positions.get_payload()?);
    assert_equals(&6, &term_positions.next_position()?);
    assert_null(&term_positions.get_payload()?.map(|p| p.to_vec()));
    assert_equals(&7, &term_positions.next_position()?);
    assert_null(&term_positions.get_payload()?.map(|p| p.to_vec()));
    close_leaf(&reader)
}

/// The anonymous `TokenStream` of `testPreAnalyzedField`.
struct PreAnalyzed {
    source: AttributeSource,
    tokens: [&'static str; 4],
    index: usize,
    term_att: Att,
}

impl std::fmt::Debug for PreAnalyzed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreAnalyzed")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl TokenStream for PreAnalyzed {
    fn increment_token(&mut self) -> Result<bool> {
        if self.index == self.tokens.len() {
            Ok(false)
        } else {
            self.source.clear_attributes();
            {
                let mut term = self.term_att.char_term_mut();
                term.set_empty();
                term.append_string(Some(self.tokens[self.index]));
            }
            self.index += 1;
            Ok(true)
        }
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// Port of `TestDocumentWriter.testPreAnalyzedField`.
fn test_pre_analyzed_field(inst: &Instance) -> Result<()> {
    let writer = IndexWriter::new(inst.dir(), mock_iwc())?;
    let mut doc = Document::new();

    let mut source =
        AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
    let term_att = Att::add(&mut source, char_term_id())?;
    let stream: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(PreAnalyzed {
        source,
        tokens: ["term1", "term2", "term3", "term2"],
        index: 0,
        term_att,
    }));
    doc.add(Box::new(TextField::new_with_token_stream(
        "preanalyzed",
        stream,
    )?));

    let info = add_commit_newest(&writer, &doc)?;
    let reader = open_segment(&info)?;

    let postings = |text: &str| -> Result<Box<dyn rucene::index::postings_enum::PostingsEnum>> {
        Ok(reader
            .postings(&Term::from_text("preanalyzed", text), POSTINGS_ENUM_ALL)?
            .unwrap_or_else(|| crate::support::npe("reader.postings")))
    };
    let mut term_positions = postings("term1")?;
    Assert::assert_true(term_positions.next_doc()? != NO_MORE_DOCS);
    assert_equals(&1, &term_positions.freq()?);
    assert_equals(&0, &term_positions.next_position()?);

    let mut term_positions = postings("term2")?;
    Assert::assert_true(term_positions.next_doc()? != NO_MORE_DOCS);
    assert_equals(&2, &term_positions.freq()?);
    assert_equals(&1, &term_positions.next_position()?);
    assert_equals(&3, &term_positions.next_position()?);

    let mut term_positions = postings("term3")?;
    Assert::assert_true(term_positions.next_doc()? != NO_MORE_DOCS);
    assert_equals(&1, &term_positions.freq()?);
    assert_equals(&2, &term_positions.next_position()?);
    close_leaf(&reader)
}

/// Test adding two fields with the same name, one indexed the other stored
/// only. The omitNorms and omitTermFreqAndPositions setting of the stored
/// field should not affect the indexed one (LUCENE-1590)
///
/// Port of `TestDocumentWriter.testLUCENE_1590`.
fn test_lucene_1590(inst: &Instance) -> Result<()> {
    let mut doc = Document::new();
    // f1 has no norms
    let custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_omit_norms(true)?;
    let custom_type2 = FieldType::new();
    custom_type2.set_stored(true)?;
    doc.add(Box::new(LuceneTestCase::new_field(
        "f1",
        "v1",
        &custom_type,
    )));
    doc.add(Box::new(LuceneTestCase::new_field(
        "f1",
        "v2",
        &custom_type2,
    )));
    // f2 has no TF
    let custom_type3 = FieldType::new_from(TextField::type_not_stored());
    custom_type3.set_index_options(IndexOptions::DOCS)?;
    let f = LuceneTestCase::new_field("f2", "v1", &custom_type3);
    doc.add(Box::new(f));
    doc.add(Box::new(LuceneTestCase::new_field(
        "f2",
        "v2",
        &custom_type2,
    )));

    let writer = IndexWriter::new(inst.dir(), mock_iwc())?;
    writer.add_document(&doc)?;
    writer.force_merge(1)?; // be sure to have a single segment
    writer.close()?;

    TestUtil::check_index(inst.dir())?;

    let reader = LuceneTestCase::get_only_leaf_reader(
        directory_reader::open(inst.dir())? as Arc<dyn IndexReader>
    )?;
    let fi = reader.get_field_infos();
    let field = |name: &str| {
        fi.field_info(name)
            .unwrap_or_else(|| crate::support::npe("fi.fieldInfo(name)"))
            .clone()
    };
    // f1
    Assert::assert_false_msg("f1 should have no norms", field("f1").has_norms());
    assert_equals_msg(
        Some("omitTermFreqAndPositions field bit should not be set for f1"),
        &IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
        &field("f1").get_index_options(),
    );
    // f2
    Assert::assert_true_msg("f2 should have norms", field("f2").has_norms());
    assert_equals_msg(
        Some("omitTermFreqAndPositions field bit should be set for f2"),
        &IndexOptions::DOCS,
        &field("f2").get_index_options(),
    );
    close_leaf(&reader)
}

/// Make sure that every new field doesn't increment memory usage by more
/// than 16kB
fn do_test_ram_usage(
    field_supplier: impl Fn(&str) -> Result<Box<dyn IndexableField>>,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_max_buffered_docs(10)?;
    iwc.set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let body = || -> Result<()> {
        let mut doc = Document::new();
        const NUM_FIELDS: i64 = 100;
        for i in 0..NUM_FIELDS {
            doc.add(field_supplier(&format!("f{i}"))?);
        }
        w.add_document(&doc)?;
        Assert::assert_true(w.has_changes_in_ram());
        assert_less_than(w.ram_bytes_used()?, NUM_FIELDS * 16384);
        Ok(())
    };
    let outcome = body();
    // try-with-resources closes `w`, then `dir`.
    let closed_w = w.close();
    let closed_dir = dir.close();
    outcome?;
    closed_w?;
    closed_dir
}

/// Port of `TestDocumentWriter.testRAMUsageStored`.
fn test_ram_usage_stored(_inst: &Instance) -> Result<()> {
    do_test_ram_usage(|field| {
        Ok(Box::new(StoredField::new_bytes(
            field,
            BytesRef::from_char_sequence("Lucene"),
        )?))
    })
}

/// Port of `TestDocumentWriter.testRAMUsageIndexed`.
fn test_ram_usage_indexed(_inst: &Instance) -> Result<()> {
    do_test_ram_usage(|field| {
        Ok(Box::new(StringField::new_with_bytes(
            field,
            BytesRef::from_char_sequence("Lucene"),
            Store::NO,
        )?))
    })
}

/// Port of `TestDocumentWriter.testRAMUsagePoint`.
fn test_ram_usage_point(_inst: &Instance) -> Result<()> {
    do_test_ram_usage(|field| Ok(Box::new(IntPoint::new(field, &[42])?)))
}

/// Port of `TestDocumentWriter.testRAMUsageNumericDocValue`.
fn test_ram_usage_numeric_doc_value(_inst: &Instance) -> Result<()> {
    do_test_ram_usage(|field| Ok(Box::new(NumericDocValuesField::new(field, 42))))
}

/// Port of `TestDocumentWriter.testRAMUsageSortedDocValue`.
fn test_ram_usage_sorted_doc_value(_inst: &Instance) -> Result<()> {
    do_test_ram_usage(|field| {
        Ok(Box::new(SortedDocValuesField::new(
            field,
            BytesRef::from_char_sequence("Lucene"),
        )))
    })
}

/// Port of `TestDocumentWriter.testRAMUsageBinaryDocValue`.
fn test_ram_usage_binary_doc_value(_inst: &Instance) -> Result<()> {
    do_test_ram_usage(|field| {
        Ok(Box::new(BinaryDocValuesField::new(
            field,
            BytesRef::from_char_sequence("Lucene"),
        )))
    })
}

/// Port of `TestDocumentWriter.testRAMUsageSortedNumericDocValue`.
fn test_ram_usage_sorted_numeric_doc_value(_inst: &Instance) -> Result<()> {
    do_test_ram_usage(|field| Ok(Box::new(SortedNumericDocValuesField::new(field, 42))))
}

/// Port of `TestDocumentWriter.testRAMUsageSortedSetDocValue`.
fn test_ram_usage_sorted_set_doc_value(_inst: &Instance) -> Result<()> {
    do_test_ram_usage(|field| {
        Ok(Box::new(SortedSetDocValuesField::new(
            field,
            BytesRef::from_char_sequence("Lucene"),
        )))
    })
}

/// Port of `TestDocumentWriter.testRAMUsageVector`.
fn test_ram_usage_vector(_inst: &Instance) -> Result<()> {
    do_test_ram_usage(|field| {
        Ok(Box::new(KnnFloatVectorField::new(
            field,
            &[1.0, 2.0, 3.0, 4.0],
            VectorSimilarityFunction::EUCLIDEAN,
        )?))
    })
}

/// `private record MockIndexableField(String field, BytesRef value,
/// IndexableFieldType fieldType) implements IndexableField`.
#[derive(Clone)]
struct MockIndexableField {
    field: String,
    value: Option<BytesRef>,
    field_type: FieldType,
}

impl std::fmt::Display for MockIndexableField {
    /// The record's `toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MockIndexableField[field={}, value={}, fieldType={}]",
            self.field,
            self.value
                .as_ref()
                .map_or_else(|| "null".to_string(), |v| v.to_string()),
            self.field_type
        )
    }
}

impl IndexableField for MockIndexableField {
    fn name(&self) -> &str {
        &self.field
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    /// Java returns `null`; the indexing chain asks a `BINARY` field for its
    /// binary value instead, so this is never called.
    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        crate::support::npe("MockIndexableField.tokenStream returned null")
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.value.clone()
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// `new FieldType()` configured by `f`, frozen.
fn frozen(f: impl FnOnce(&mut FieldType) -> Result<()>) -> Result<FieldType> {
    let mut t = FieldType::new();
    f(&mut t)?;
    t.freeze();
    Ok(t)
}

/// `new IndexWriter(dir, newIndexWriterConfig().setOpenMode(OpenMode.CREATE))`.
fn create_writer(inst: &Instance) -> Result<Arc<IndexWriter>> {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_open_mode(OpenMode::CREATE)?;
    IndexWriter::new(inst.dir(), iwc)
}

/// Port of `TestDocumentWriter.testIndexBinaryValueWithoutTokenStream`.
fn test_index_binary_value_without_token_stream(inst: &Instance) -> Result<()> {
    let illegal_field_types = vec![
        frozen(|t| {
            // cannot index a tokenized binary field
            t.set_tokenized(true)?;
            t.set_index_options(IndexOptions::DOCS)
        })?,
        frozen(|t| {
            t.set_tokenized(false)?;
            // cannot index positions on a binary field
            t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        })?,
        frozen(|t| {
            t.set_tokenized(false)?;
            t.set_index_options(IndexOptions::DOCS)?;
            t.set_store_term_vectors(true)?;
            // cannot index term vector positions
            t.set_store_term_vector_positions(true)
        })?,
        frozen(|t| {
            t.set_tokenized(false)?;
            t.set_index_options(IndexOptions::DOCS)?;
            t.set_store_term_vectors(true)?;
            // cannot index term vector offsets
            t.set_store_term_vector_offsets(true)
        })?,
    ];

    for ft in &illegal_field_types {
        let w = create_writer(inst)?;
        let field = MockIndexableField {
            field: "field".to_string(),
            value: Some(BytesRef::from_char_sequence("a")),
            field_type: ft.clone(),
        };
        let mut doc = Document::new();
        doc.add(Box::new(field));
        expect_throws(IAE, || w.add_document(&doc));
        w.close()?;
    }

    {
        let w = create_writer(inst)?;
        // Field that has both a null token stream and a null binary value
        let field = MockIndexableField {
            field: "field".to_string(),
            value: None,
            field_type: FieldType::new_from(StringField::type_not_stored()),
        };
        let mut doc = Document::new();
        doc.add(Box::new(field));
        expect_throws(IAE, || w.add_document(&doc));
        w.close()?;
    }

    let legal_field_types = vec![
        frozen(|t| {
            t.set_tokenized(false)?;
            t.set_index_options(IndexOptions::DOCS)?;
            t.set_omit_norms(false)
        })?,
        frozen(|t| {
            t.set_tokenized(false)?;
            t.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
            t.set_omit_norms(false)
        })?,
        frozen(|t| {
            t.set_tokenized(false)?;
            t.set_index_options(IndexOptions::DOCS)?;
            t.set_omit_norms(true)
        })?,
        frozen(|t| {
            t.set_tokenized(false)?;
            t.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
            t.set_omit_norms(true)
        })?,
        frozen(|t| {
            t.set_tokenized(false)?;
            t.set_index_options(IndexOptions::DOCS)?;
            t.set_store_term_vectors(true)
        })?,
        frozen(|t| {
            t.set_tokenized(false)?;
            t.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
            t.set_store_term_vectors(true)
        })?,
    ];

    for ft in &legal_field_types {
        {
            let w = create_writer(inst)?;
            let field = MockIndexableField {
                field: "field".to_string(),
                value: Some(BytesRef::from_char_sequence("a")),
                field_type: ft.clone(),
            };
            let mut doc = Document::new();
            // Java adds the same field object twice.
            doc.add(Box::new(field.clone()));
            doc.add(Box::new(field));
            w.add_document(&doc)?;
            w.close()?;
        }

        let reader = directory_reader::open(inst.dir())?;
        {
            let leaf_reader =
                LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

            {
                let terms = leaf_reader
                    .terms("field")?
                    .unwrap_or_else(|| crate::support::npe("leafReader.terms(\"field\")"));
                assert_equals(&1, &terms.sum_doc_freq()?);
                if ft.index_options().subsumes(IndexOptions::DOCS_AND_FREQS) {
                    assert_equals(&2, &terms.sum_total_term_freq()?);
                } else {
                    assert_equals(&1, &terms.sum_total_term_freq()?);
                }
                let mut terms_enum = terms.iterator()?;
                Assert::assert_true(terms_enum.seek_exact(&BytesRef::from_char_sequence("a"))?);
                let mut pe = terms_enum
                    .postings(None, POSTINGS_ENUM_ALL)?
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                assert_equals(&0, &pe.next_doc()?);
                if ft.index_options().subsumes(IndexOptions::DOCS_AND_FREQS) {
                    assert_equals(&2, &pe.freq()?);
                } else {
                    assert_equals(&1, &pe.freq()?);
                }
                assert_equals(&-1, &pe.next_position()?);
                assert_equals(&NO_MORE_DOCS, &pe.next_doc()?);
            }

            if ft.store_term_vectors() {
                let tv_terms = leaf_reader
                    .term_vectors()?
                    .get(0)?
                    .unwrap_or_else(|| crate::support::npe("termVectors().get(0)"))
                    .terms("field")?
                    .unwrap_or_else(|| crate::support::npe("terms(\"field\")"));
                assert_equals(&1, &tv_terms.sum_doc_freq()?);
                assert_equals(&2, &tv_terms.sum_total_term_freq()?);
                let mut tv_terms_enum = tv_terms.iterator()?;
                Assert::assert_true(tv_terms_enum.seek_exact(&BytesRef::from_char_sequence("a"))?);
                let mut pe = tv_terms_enum
                    .postings(None, POSTINGS_ENUM_ALL)?
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                assert_equals(&0, &pe.next_doc()?);
                assert_equals(&2, &pe.freq()?);
                assert_equals(&-1, &pe.next_position()?);
                assert_equals(&NO_MORE_DOCS, &pe.next_doc()?);
            } else {
                Assert::assert_true(leaf_reader.term_vectors()?.get(0)?.is_none());
            }
        }
        reader.close()?;
    }
    Ok(())
}
