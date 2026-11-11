//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTermVectorsReader.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, TokenStreamComponents, Tokenizer,
    TokenizerLogic,
};
use rucene::codecs;
use rucene::codecs::term_vectors::TermVectorsReader;
use rucene::document::{Document, Field, FieldType, StoredField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::field_infos::FieldInfos;
use rucene::index::index_writer::IndexWriter;
use rucene::index::postings_enum::{
    PostingsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
};
use rucene::index::{IndexReader, SegmentCommitInfo};
use rucene::search::NO_MORE_DOCS;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, pos_inc_id, Att};
use rucene_test_framework::analysis::{tokenizer_components, MockAnalyzer};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, assert_null, iae_message, random};
use crate::support::npe;

/// `private static final int TERM_FREQ = 3`.
const TERM_FREQ: usize = 3;

/// `private static class TestToken implements Comparable<TestToken>`.
#[derive(Debug, Clone, Default)]
struct TestToken {
    text: String,
    pos: i32,
    start_offset: i32,
    end_offset: i32,
}

/// The fields of `TestTermVectorsReader`, set by their initializers and
/// `setUp()`.
struct TestTermVectorsReaderInstance {
    // Must be lexicographically sorted, will do in setup, versus trying to maintain here
    test_fields: [&'static str; 4],
    test_fields_store_pos: [bool; 4],
    test_fields_store_off: [bool; 4],
    test_terms: Vec<String>,
    positions: Vec<Vec<i32>>,
    dir: Option<TestDirectory>,
    seg: Option<SegmentCommitInfo>,
    field_infos: FieldInfos,
    tokens: Arc<Vec<TestToken>>,
}

impl Default for TestTermVectorsReaderInstance {
    fn default() -> Self {
        let test_terms: Vec<String> = ["this", "is", "a", "test"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        Self {
            test_fields: ["f1", "f2", "f3", "f4"],
            test_fields_store_pos: [true, false, true, false],
            test_fields_store_off: [true, false, false, true],
            positions: vec![Vec::new(); test_terms.len()],
            tokens: Arc::new(vec![TestToken::default(); test_terms.len() * TERM_FREQ]),
            test_terms,
            dir: None,
            seg: None,
            field_infos: FieldInfos::empty(),
        }
    }
}

impl TestTermVectorsReaderInstance {
    fn dir(&self) -> &TestDirectory {
        self.dir.as_ref().unwrap_or_else(|| npe("dir"))
    }

    fn seg(&self) -> &SegmentCommitInfo {
        self.seg.as_ref().unwrap_or_else(|| npe("seg"))
    }

    /// `Codec.getDefault().termVectorsFormat().vectorsReader(dir, seg.info, fieldInfos,
    /// newIOContext(random()))`.
    fn vectors_reader(&self) -> Result<Box<dyn TermVectorsReader>> {
        codecs::get_default()
            .unwrap_or_else(|| npe("Codec.getDefault()"))
            .term_vectors_format()
            .vectors_reader(
                &*self.dir().as_directory(),
                &self.seg().info,
                &self.field_infos,
                &*LuceneTestCase::new_io_context(&*random()),
            )
    }
}

/// A method of the class over its instance.
fn method(name: &str, body: fn(&mut TestTermVectorsReaderInstance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut TestTermVectorsReaderInstance| {
            body(inst).unwrap_or_else(|e| rethrow_java(e))
        },
    )
}

/// A test method that does not read the instance.
fn own(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |_inst: &mut TestTermVectorsReaderInstance| body().unwrap_or_else(|e| rethrow_java(e)),
    )
}

/// `TestTermVectorsReader`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestTermVectorsReader")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestTermVectorsReaderInstance::default))
        .method(method("setUp", set_up))
        .method(method("tearDown", tear_down))
        .method(method("test", test_))
        .method(method("testReader", test_reader))
        .method(method("testDocsEnum", test_docs_enum))
        .method(method("testPositionReader", test_position_reader))
        .method(method("testOffsetReader", test_offset_reader))
        .method(own(
            "testIllegalPayloadsWithoutPositions",
            test_illegal_payloads_without_positions,
        ))
        .method(own(
            "testIllegalOffsetsWithoutVectors",
            test_illegal_offsets_without_vectors,
        ))
        .method(own(
            "testIllegalPositionsWithoutVectors",
            test_illegal_positions_without_vectors,
        ))
        .method(own(
            "testIllegalVectorPayloadsWithoutVectors",
            test_illegal_vector_payloads_without_vectors,
        ))
        .method(own(
            "testIllegalVectorsWithoutIndexed",
            test_illegal_vectors_without_indexed,
        ))
        .method(own(
            "testIllegalVectorPositionsWithoutIndexed",
            test_illegal_vector_positions_without_indexed,
        ))
        .method(own(
            "testIllegalVectorOffsetsWithoutIndexed",
            test_illegal_vector_offsets_without_indexed,
        ))
        .method(own(
            "testIllegalVectorPayloadsWithoutIndexed",
            test_illegal_vector_payloads_without_indexed,
        ))
        .build()
}

/// `setUp()`.
fn set_up(inst: &mut TestTermVectorsReaderInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    /*
    for (int i = 0; i < testFields.length; i++) {
      fieldInfos.add(testFields[i], true, true, testFieldsStorePos[i], testFieldsStoreOff[i]);
    }
    */

    inst.test_terms.sort();
    let mut tokens: Vec<TestToken> = Vec::with_capacity(inst.test_terms.len() * TERM_FREQ);
    let rnd = random();
    for i in 0..inst.test_terms.len() {
        inst.positions[i] = vec![0; TERM_FREQ];
        // first position must be 0
        for j in 0..TERM_FREQ {
            // positions are always sorted in increasing order
            inst.positions[i][j] = (j as f64 * 10.0 + rnd.next_double() * 10.0) as i32;
            let len = inst.test_terms[i].encode_utf16().count() as i32;
            tokens.push(TestToken {
                text: inst.test_terms[i].clone(),
                pos: inst.positions[i][j],
                start_offset: j as i32 * 10,
                end_offset: j as i32 * 10 + len,
            });
        }
    }
    // `Arrays.sort(tokens)`: stable, by `pos`.
    tokens.sort_by_key(|t| t.pos);
    inst.tokens = Arc::new(tokens);

    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MyAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        tokens: Arc::clone(&inst.tokens),
    })));
    iwc.set_max_buffered_docs(-1)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 10)?,
    )))?;
    iwc.set_use_compound_file(false)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    for i in 0..inst.test_fields.len() {
        let custom_type = FieldType::new_from(TextField::type_not_stored());
        let (pos, off) = (inst.test_fields_store_pos[i], inst.test_fields_store_off[i]);
        if pos && off {
            custom_type.set_store_term_vectors(true)?;
            custom_type.set_store_term_vector_positions(true)?;
            custom_type.set_store_term_vector_offsets(true)?;
        } else if pos && !off {
            custom_type.set_store_term_vectors(true)?;
            custom_type.set_store_term_vector_positions(true)?;
        } else if !pos && off {
            custom_type.set_store_term_vectors(true)?;
            custom_type.set_store_term_vector_positions(true)?;
            custom_type.set_store_term_vector_offsets(true)?;
        } else {
            custom_type.set_store_term_vectors(true)?;
        }
        doc.add(Box::new(Field::new(
            inst.test_fields[i],
            String::new(),
            custom_type,
        )?));
    }

    // Create 5 documents for testing, they all have the same
    // terms
    for _ in 0..5 {
        writer.add_document(&doc)?;
    }
    writer.commit()?;
    inst.seg = writer.newest_segment()?;
    writer.close()?;
    inst.dir = Some(dir);

    inst.field_infos = IndexWriter::read_field_infos(inst.seg())?;
    Ok(())
}

/// `tearDown()`.
fn tear_down(inst: &mut TestTermVectorsReaderInstance) -> Result<()> {
    inst.dir.take().unwrap_or_else(|| npe("dir")).close()?;
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// `private class MyTokenizer extends Tokenizer`.
#[derive(Debug)]
struct MyTokenizer {
    tokens: Arc<Vec<TestToken>>,
    token_upto: usize,
    term_att: Att,
    pos_incr_att: Att,
    offset_att: Att,
}

impl TokenizerLogic for MyTokenizer {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _input: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.token_upto >= self.tokens.len() {
            Ok(false)
        } else {
            let test_token = &self.tokens[self.token_upto];
            self.token_upto += 1;
            source.clear_attributes();
            self.term_att
                .char_term_mut()
                .append_string(Some(&test_token.text));
            self.offset_att
                .offset_mut()
                .set_offset(test_token.start_offset, test_token.end_offset);
            if self.token_upto > 1 {
                self.pos_incr_att
                    .pos_inc_mut()
                    .set_position_increment(test_token.pos - self.tokens[self.token_upto - 2].pos);
            } else {
                self.pos_incr_att
                    .pos_inc_mut()
                    .set_position_increment(test_token.pos + 1);
            }
            Ok(true)
        }
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.token_upto = 0;
        Ok(())
    }
}

/// `private class MyAnalyzer extends Analyzer`.
#[derive(Debug)]
struct MyAnalyzer {
    state: AnalyzerState,
    tokens: Arc<Vec<TestToken>>,
}

impl Analyzer for MyAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestTermVectorsReader$MyAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let pos_incr_att = Att::add(&mut source, pos_inc_id())?;
        let offset_att = Att::add(&mut source, offset_id())?;
        let t = Tokenizer::with_source(
            source,
            MyTokenizer {
                tokens: Arc::clone(&self.tokens),
                token_upto: 0,
                term_att,
                pos_incr_att,
                offset_att,
            },
        );
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(t))))
    }
}

/// Port of `TestTermVectorsReader.test`.
fn test_(inst: &mut TestTermVectorsReaderInstance) -> Result<()> {
    // Check to see the files were created properly in setup
    let reader = directory_reader::open(inst.dir().as_directory())?;
    for ctx in (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()? {
        let leaf = ctx.leaf_reader();
        let sr = leaf.as_segment_reader().unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.ClassCastException",
                "not a SegmentReader",
            )
            .throw()
        });
        Assert::assert_true(rucene::index::LeafReader::get_field_infos(sr).has_term_vectors());
    }
    reader.close()
}

/// `reader.get(j).terms(field)`.
fn get_vector(
    reader: &dyn TermVectorsReader,
    doc: i32,
    field: &str,
) -> Result<Option<Box<dyn rucene::index::Terms>>> {
    reader
        .get(doc)?
        .unwrap_or_else(|| npe("reader.get(j)"))
        .terms(field)
}

/// `text.utf8ToString()` of a non-null `termsEnum.next()`.
fn next_term(terms_enum: &mut dyn rucene::index::terms::TermsEnum) -> Result<String> {
    let text = terms_enum.next()?;
    assert_not_null(&text);
    text.expect("checked").utf8_to_string()
}

/// Port of `TestTermVectorsReader.testReader`.
fn test_reader(inst: &mut TestTermVectorsReaderInstance) -> Result<()> {
    let mut reader = inst.vectors_reader()?;
    for j in 0..5 {
        let vector = get_vector(&*reader, j, inst.test_fields[0])?;
        assert_not_null(&vector);
        let vector = vector.expect("checked");
        assert_equals(&(inst.test_terms.len() as i64), &vector.size()?);
        let mut terms_enum = vector.iterator()?;
        for i in 0..inst.test_terms.len() {
            let term = next_term(&mut *terms_enum)?;
            // System.out.println("Term: " + term);
            assert_equals(&inst.test_terms[i], &term);
        }
        assert_null(&terms_enum.next()?);
    }
    reader.close()
}

/// Port of `TestTermVectorsReader.testDocsEnum`.
fn test_docs_enum(inst: &mut TestTermVectorsReaderInstance) -> Result<()> {
    let mut reader = inst.vectors_reader()?;
    for j in 0..5 {
        let vector = get_vector(&*reader, j, inst.test_fields[0])?;
        assert_not_null(&vector);
        let vector = vector.expect("checked");
        assert_equals(&(inst.test_terms.len() as i64), &vector.size()?);
        let mut terms_enum = vector.iterator()?;
        let mut postings_enum: Option<Box<dyn PostingsEnum>> = None;
        for i in 0..inst.test_terms.len() {
            let term = next_term(&mut *terms_enum)?;
            // System.out.println("Term: " + term);
            assert_equals(&inst.test_terms[i], &term);

            let pe = TestUtil::docs_of(
                &*random(),
                &mut *terms_enum,
                postings_enum.take(),
                POSTINGS_ENUM_NONE,
            )?;
            assert_not_null(&pe);
            let mut pe = pe.ok_or_else(rucene::error::LuceneError::null_pointer)?;
            let doc = pe.doc_id();
            assert_equals(&-1, &doc);
            Assert::assert_true(pe.next_doc()? != NO_MORE_DOCS);
            assert_equals(&NO_MORE_DOCS, &pe.next_doc()?);
            postings_enum = Some(pe);
        }
        assert_null(&terms_enum.next()?);
    }
    reader.close()
}

/// Port of `TestTermVectorsReader.testPositionReader`.
fn test_position_reader(inst: &mut TestTermVectorsReaderInstance) -> Result<()> {
    let mut reader = inst.vectors_reader()?;
    let vector = get_vector(&*reader, 0, inst.test_fields[0])?;
    assert_not_null(&vector);
    let vector = vector.expect("checked");
    assert_equals(&(inst.test_terms.len() as i64), &vector.size()?);
    let mut terms_enum = vector.iterator()?;
    let mut dp_enum: Option<Box<dyn PostingsEnum>> = None;
    for i in 0..inst.test_terms.len() {
        let term = next_term(&mut *terms_enum)?;
        // System.out.println("Term: " + term);
        assert_equals(&inst.test_terms[i], &term);

        let dp = terms_enum.postings(dp_enum.take(), POSTINGS_ENUM_ALL)?;
        assert_not_null(&dp);
        let mut dp = dp.ok_or_else(rucene::error::LuceneError::null_pointer)?;
        let doc = dp.doc_id();
        assert_equals(&-1, &doc);
        Assert::assert_true(dp.next_doc()? != NO_MORE_DOCS);
        assert_equals(&(inst.positions[i].len() as i32), &dp.freq()?);
        for j in 0..inst.positions[i].len() {
            assert_equals(&inst.positions[i][j], &dp.next_position()?);
        }
        assert_equals(&NO_MORE_DOCS, &dp.next_doc()?);

        let mut dp = terms_enum
            .postings(Some(dp), POSTINGS_ENUM_ALL)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        let doc = dp.doc_id();
        assert_equals(&-1, &doc);
        Assert::assert_true(dp.next_doc()? != NO_MORE_DOCS);
        assert_equals(&(inst.positions[i].len() as i32), &dp.freq()?);
        for j in 0..inst.positions[i].len() {
            assert_equals(&inst.positions[i][j], &dp.next_position()?);
            assert_equals(&(j as i32 * 10), &dp.start_offset());
            assert_equals(
                &(j as i32 * 10 + inst.test_terms[i].encode_utf16().count() as i32),
                &dp.end_offset(),
            );
        }
        assert_equals(&NO_MORE_DOCS, &dp.next_doc()?);
        dp_enum = Some(dp);
    }

    let freq_vector = get_vector(&*reader, 0, inst.test_fields[1])?; // no pos, no offset
    assert_not_null(&freq_vector);
    let freq_vector = freq_vector.expect("checked");
    assert_equals(&(inst.test_terms.len() as i64), &freq_vector.size()?);
    let mut terms_enum = freq_vector.iterator()?;
    for i in 0..inst.test_terms.len() {
        let term = next_term(&mut *terms_enum)?;
        // System.out.println("Term: " + term);
        assert_equals(&inst.test_terms[i], &term);
        // `postings(null)`: `postings(null, PostingsEnum.FREQS)`.
        assert_not_null(&terms_enum.postings(None, POSTINGS_ENUM_FREQS)?);
        assert_not_null(&terms_enum.postings(None, POSTINGS_ENUM_ALL)?);
    }
    reader.close()
}

/// Port of `TestTermVectorsReader.testOffsetReader`.
fn test_offset_reader(inst: &mut TestTermVectorsReaderInstance) -> Result<()> {
    let mut reader = inst.vectors_reader()?;
    let vector = get_vector(&*reader, 0, inst.test_fields[0])?;
    assert_not_null(&vector);
    let vector = vector.expect("checked");
    let mut terms_enum = vector.iterator()?;
    assert_equals(&(inst.test_terms.len() as i64), &vector.size()?);
    let mut dp_enum: Option<Box<dyn PostingsEnum>> = None;
    for i in 0..inst.test_terms.len() {
        let term = next_term(&mut *terms_enum)?;
        assert_equals(&inst.test_terms[i], &term);

        let dp = terms_enum.postings(dp_enum.take(), POSTINGS_ENUM_ALL)?;
        assert_not_null(&dp);
        let mut dp = dp.ok_or_else(rucene::error::LuceneError::null_pointer)?;
        Assert::assert_true(dp.next_doc()? != NO_MORE_DOCS);
        assert_equals(&(inst.positions[i].len() as i32), &dp.freq()?);
        for j in 0..inst.positions[i].len() {
            assert_equals(&inst.positions[i][j], &dp.next_position()?);
        }
        assert_equals(&NO_MORE_DOCS, &dp.next_doc()?);

        let mut dp = terms_enum
            .postings(Some(dp), POSTINGS_ENUM_ALL)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        Assert::assert_true(dp.next_doc()? != NO_MORE_DOCS);
        assert_equals(&(inst.positions[i].len() as i32), &dp.freq()?);
        for j in 0..inst.positions[i].len() {
            assert_equals(&inst.positions[i][j], &dp.next_position()?);
            assert_equals(&(j as i32 * 10), &dp.start_offset());
            assert_equals(
                &(j as i32 * 10 + inst.test_terms[i].encode_utf16().count() as i32),
                &dp.end_offset(),
            );
        }
        assert_equals(&NO_MORE_DOCS, &dp.next_doc()?);
        dp_enum = Some(dp);
    }
    reader.close()
}

/// The shared body of the `testIllegal*` tests: indexing `"value"` in a
/// field of `ft` must throw an `IllegalArgumentException` with `message`.
fn check_illegal(ft: FieldType, message: &str) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let a = MockAnalyzer::new(&*random());
    a.set_enable_checks(false);
    let w = RandomIndexWriter::new(&*random(), dir.as_directory(), Arc::new(a))?;
    let mut doc = Document::new();
    doc.add(Box::new(Field::new("field", "value".to_string(), ft)?));

    let expected = iae_message(|| w.add_document(&doc));
    assert_equals(message, expected.as_str());

    w.close()?;
    dir.close()
}

/// Port of `TestTermVectorsReader.testIllegalPayloadsWithoutPositions`.
fn test_illegal_payloads_without_positions() -> Result<()> {
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_store_term_vectors(true)?;
    ft.set_store_term_vector_payloads(true)?;
    check_illegal(
        ft,
        "cannot index term vector payloads without term vector positions (field=\"field\")",
    )
}

/// Port of `TestTermVectorsReader.testIllegalOffsetsWithoutVectors`.
fn test_illegal_offsets_without_vectors() -> Result<()> {
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_store_term_vectors(false)?;
    ft.set_store_term_vector_offsets(true)?;
    check_illegal(
        ft,
        "cannot index term vector offsets when term vectors are not indexed (field=\"field\")",
    )
}

/// Port of `TestTermVectorsReader.testIllegalPositionsWithoutVectors`.
fn test_illegal_positions_without_vectors() -> Result<()> {
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_store_term_vectors(false)?;
    ft.set_store_term_vector_positions(true)?;
    check_illegal(
        ft,
        "cannot index term vector positions when term vectors are not indexed (field=\"field\")",
    )
}

/// Port of `TestTermVectorsReader.testIllegalVectorPayloadsWithoutVectors`.
fn test_illegal_vector_payloads_without_vectors() -> Result<()> {
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_store_term_vectors(false)?;
    ft.set_store_term_vector_payloads(true)?;
    check_illegal(
        ft,
        "cannot index term vector payloads when term vectors are not indexed (field=\"field\")",
    )
}

/// Port of `TestTermVectorsReader.testIllegalVectorsWithoutIndexed`.
fn test_illegal_vectors_without_indexed() -> Result<()> {
    let ft = FieldType::new_from(StoredField::type_());
    ft.set_store_term_vectors(true)?;
    check_illegal(
        ft,
        "cannot store term vectors for a field that is not indexed (field=\"field\")",
    )
}

/// Port of `TestTermVectorsReader.testIllegalVectorPositionsWithoutIndexed`.
fn test_illegal_vector_positions_without_indexed() -> Result<()> {
    let ft = FieldType::new_from(StoredField::type_());
    ft.set_store_term_vector_positions(true)?;
    check_illegal(
        ft,
        "cannot store term vector positions for a field that is not indexed (field=\"field\")",
    )
}

/// Port of `TestTermVectorsReader.testIllegalVectorOffsetsWithoutIndexed`.
fn test_illegal_vector_offsets_without_indexed() -> Result<()> {
    let ft = FieldType::new_from(StoredField::type_());
    ft.set_store_term_vector_offsets(true)?;
    check_illegal(
        ft,
        "cannot store term vector offsets for a field that is not indexed (field=\"field\")",
    )
}

/// Port of `TestTermVectorsReader.testIllegalVectorPayloadsWithoutIndexed`.
fn test_illegal_vector_payloads_without_indexed() -> Result<()> {
    let ft = FieldType::new_from(StoredField::type_());
    ft.set_store_term_vector_payloads(true)?;
    check_illegal(
        ft,
        "cannot store term vector payloads for a field that is not indexed (field=\"field\")",
    )
}
