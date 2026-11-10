//! Port of `lucene/core/src/test/org/apache/lucene/index/TestCustomTermFreq.java`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{default_token_attribute_factory, TokenStream};
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_FREQS};
use rucene::index::{FieldInvertState, IndexOptions, IndexReader};
use rucene::search::similarities::{CollectionStatistics, SimScorer, Similarity, TermStatistics};
use rucene::search::NO_MORE_DOCS;
use rucene::util::attribute::AttributeSource;
use rucene_test_framework::analysis::attributes::{char_term_id, term_freq_id, Att};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_not_null, class, random, test, IAE};

/// `TestCustomTermFreq`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestCustomTermFreq",
        vec![
            test("testSingletonTermsOneDoc", test_singleton_terms_one_doc),
            test("testRepeatedTerms", test_repeated_terms),
            test("testSingletonTermsTwoDocs", test_singleton_terms_two_docs),
            test("testRepeatTermsOneDoc", test_repeat_terms_one_doc),
            test("testRepeatTermsTwoDocs", test_repeat_terms_two_docs),
            test("testTotalTermFreq", test_total_term_freq),
            test("testTotalTermFreqTermDoc", test_total_term_freq_term_doc),
            test("testInvalidProx", test_invalid_prox),
            test("testInvalidDocsOnly", test_invalid_docs_only),
            test("testOverflowInt", test_overflow_int),
            test("testNoOverflowInt", test_no_overflow_int),
            test(
                "testInvalidTermVectorPositions",
                test_invalid_term_vector_positions,
            ),
            test(
                "testInvalidTermVectorOffsets",
                test_invalid_term_vector_offsets,
            ),
            test("testTermVectors", test_term_vectors),
            test("testFieldInvertState", test_field_invert_state),
        ],
    )
}

const ISE: &str = "java.lang.IllegalStateException";

/// `private static final class CannedTermFreqs extends TokenStream`.
struct CannedTermFreqs {
    source: AttributeSource,
    terms: Vec<String>,
    term_freqs: Vec<i32>,
    term_att: Att,
    term_freq_att: Att,
    upto: usize,
}

impl CannedTermFreqs {
    fn new(terms: &[&str], term_freqs: &[i32]) -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let term_freq_att = Att::add(&mut source, term_freq_id())?;
        debug_assert!(terms.len() == term_freqs.len());
        Ok(CannedTermFreqs {
            source,
            terms: terms.iter().map(|t| t.to_string()).collect(),
            term_freqs: term_freqs.to_vec(),
            term_att,
            term_freq_att,
            upto: 0,
        })
    }
}

impl std::fmt::Debug for CannedTermFreqs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CannedTermFreqs")
            .field("terms", &self.terms)
            .field("term_freqs", &self.term_freqs)
            .field("upto", &self.upto)
            .finish_non_exhaustive()
    }
}

impl TokenStream for CannedTermFreqs {
    fn increment_token(&mut self) -> Result<bool> {
        if self.upto == self.terms.len() {
            return Ok(false);
        }

        self.source.clear_attributes();

        // `termAtt.append(terms[upto])` on the cleared term.
        self.term_att.set_term(&self.terms[self.upto]);
        self.term_freq_att
            .term_freq_mut()
            .set_term_frequency(self.term_freqs[self.upto]);

        self.upto += 1;
        Ok(true)
    }

    fn reset(&mut self) -> Result<()> {
        self.upto = 0;
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// `new Field(name, new CannedTermFreqs(terms, termFreqs), fieldType)`.
fn canned_field(
    name: &str,
    terms: &[&str],
    term_freqs: &[i32],
    field_type: &FieldType,
) -> Result<Field> {
    let stream: Rc<RefCell<dyn TokenStream>> =
        Rc::new(RefCell::new(CannedTermFreqs::new(terms, term_freqs)?));
    Field::new_with_token_stream(name, stream, field_type.clone())
}

/// A document holding `field`.
fn doc_with(field: Field) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(field));
    doc
}

/// `Directory dir = newDirectory(); IndexWriter w = new IndexWriter(dir, new
/// IndexWriterConfig(new MockAnalyzer(random())));`
fn open() -> Result<(TestDirectory, Arc<IndexWriter>)> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
    )?;
    Ok((dir, w))
}

/// `IOUtils.close(r, w, dir)` (`r` absent when the test opened none); the
/// try-with-resources of `dir` and `w` closes them in the same order.
fn close(
    r: Option<Arc<dyn IndexReader>>,
    w: Arc<IndexWriter>,
    mut dir: TestDirectory,
) -> Result<()> {
    if let Some(r) = r {
        r.close()?;
    }
    w.close()?;
    dir.close()
}

/// `DirectoryReader.open(w)`.
fn reader(w: &IndexWriter) -> Result<Arc<dyn IndexReader>> {
    Ok(directory_reader::open_with_writer(w)?)
}

/// `MultiTerms.getTermPostingsEnum(r, "field", newBytesRef(term), (int)
/// PostingsEnum.FREQS)`, checked `assertNotNull`.
fn postings(r: &Arc<dyn IndexReader>, term: &str) -> Result<Box<dyn PostingsEnum>> {
    let postings = MultiTerms::get_term_postings_enum(
        r,
        "field",
        &LuceneTestCase::new_bytes_ref_str(term),
        POSTINGS_ENUM_FREQS,
    )?;
    assert_not_null(&postings);
    Ok(postings.expect("checked by assertNotNull"))
}

/// The `nextDoc`/`freq` checks of one postings list: each `(doc, freq)`,
/// then `NO_MORE_DOCS`.
fn assert_postings(postings: &mut dyn PostingsEnum, expected: &[(i32, i32)]) -> Result<()> {
    for &(doc, freq) in expected {
        assert_equals(&doc, &postings.next_doc()?);
        assert_equals(&freq, &postings.freq()?);
    }
    assert_equals(&NO_MORE_DOCS, &postings.next_doc()?);
    Ok(())
}

/// `MultiTerms.getTerms(r, "field").iterator()`.
fn terms_enum(r: &Arc<dyn IndexReader>) -> Result<Box<dyn rucene::index::terms::TermsEnum>> {
    MultiTerms::get_terms(r, "field")?
        .unwrap_or_else(|| crate::support::npe("MultiTerms.getTerms(r, \"field\")"))
        .iterator()
}

/// Port of `TestCustomTermFreq.testSingletonTermsOneDoc`.
fn test_singleton_terms_one_doc() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = get_term_doc_field_type()?;
    let field = canned_field("field", &["foo", "bar"], &[42, 128], &field_type)?;
    let doc = doc_with(field);
    w.add_document(&doc)?;
    {
        let r = reader(&w)?;
        let mut p = postings(&r, "bar")?;
        assert_postings(&mut *p, &[(0, 128)])?;

        let mut p = postings(&r, "foo")?;
        assert_postings(&mut *p, &[(0, 42)])?;
        r.close()?;
    }
    close(None, w, dir)
}

/// Port of `TestCustomTermFreq.testRepeatedTerms`.
fn test_repeated_terms() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = get_term_doc_field_type()?;
    let field = canned_field("field", &["foo", "foo"], &[42, 128], &field_type)?;
    let doc = doc_with(field);
    let e = expect_throws(IAE, || w.add_document(&doc));
    assert_equals(
        &Some("Document update skipped due to duplicate termdoc term"),
        &e.message(),
    );
    // `(TermsHashPerField.DuplicateTermException) e.getCause()`
    let dte = e
        .cause()
        .unwrap_or_else(|| crate::support::npe("e.getCause()"));
    assert_equals(
        &Some("field 'field' has duplicate term 'foo'"),
        &dte.message(),
    );

    // ensure IndexWriter remains open and doc was skipped
    let field = canned_field("field", &["foo"], &[42], &field_type)?;
    let ok_doc = doc_with(field);
    w.add_document(&ok_doc)?;
    {
        let r = reader(&w)?;
        assert_equals(&1, &r.num_docs());
        r.close()?;
    }
    close(None, w, dir)
}

/// `private static FieldType getTermDocFieldType()`.
fn get_term_doc_field_type() -> Result<FieldType> {
    let field_type = FieldType::new_from(TextField::type_not_stored());
    field_type.set_index_options(IndexOptions::DOCS_AND_CUSTOM_FREQS)?;
    Ok(field_type)
}

/// `private static FieldType getLegacyFieldType()`.
fn get_legacy_field_type() -> Result<FieldType> {
    // It is possible to store custom term frequencies in fields that are *not* labeled as term-doc
    // fields. These will sum term frequencies rather than counts and have more places where they
    // may overflow these aggregates.
    let field_type = FieldType::new_from(TextField::type_not_stored());
    field_type.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
    Ok(field_type)
}

/// Port of `TestCustomTermFreq.testSingletonTermsTwoDocs`.
fn test_singleton_terms_two_docs() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = get_term_doc_field_type()?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar"],
        &[42, 128],
        &field_type,
    )?);
    w.add_document(&doc)?;

    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar"],
        &[50, 50],
        &field_type,
    )?);
    w.add_document(&doc)?;

    let r = reader(&w)?;
    let mut p = postings(&r, "bar")?;
    assert_postings(&mut *p, &[(0, 128), (1, 50)])?;

    let mut p = postings(&r, "foo")?;
    assert_postings(&mut *p, &[(0, 42), (1, 50)])?;

    close(Some(r), w, dir)
}

/// Port of `TestCustomTermFreq.testRepeatTermsOneDoc`.
fn test_repeat_terms_one_doc() -> Result<()> {
    // With legacy field type, we sum the individual term freqs
    let (dir, w) = open()?;
    let field_type = get_legacy_field_type()?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar", "foo", "bar"],
        &[42, 128, 17, 100],
        &field_type,
    )?);
    w.add_document(&doc)?;
    {
        let r = reader(&w)?;
        let mut p = postings(&r, "bar")?;
        assert_postings(&mut *p, &[(0, 228)])?;

        let mut p = postings(&r, "foo")?;
        assert_postings(&mut *p, &[(0, 59)])?;
        r.close()?;
    }
    close(None, w, dir)
}

/// Port of `TestCustomTermFreq.testRepeatTermsTwoDocs`.
fn test_repeat_terms_two_docs() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = get_legacy_field_type()?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar", "foo", "bar"],
        &[42, 128, 17, 100],
        &field_type,
    )?);
    w.add_document(&doc)?;

    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar", "foo", "bar"],
        &[50, 60, 70, 80],
        &field_type,
    )?);
    w.add_document(&doc)?;

    let r = reader(&w)?;
    let mut p = postings(&r, "bar")?;
    assert_postings(&mut *p, &[(0, 228), (1, 140)])?;

    let mut p = postings(&r, "foo")?;
    assert_postings(&mut *p, &[(0, 59), (1, 120)])?;

    close(Some(r), w, dir)
}

/// Port of `TestCustomTermFreq.testTotalTermFreq`.
fn test_total_term_freq() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = get_legacy_field_type()?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar", "foo", "bar", "baz"],
        &[42, 128, 17, 100, 99],
        &field_type,
    )?);
    w.add_document(&doc)?;

    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar", "foo", "bar"],
        &[50, 60, 70, 80],
        &field_type,
    )?);
    w.add_document(&doc)?;

    let r = reader(&w)?;

    {
        let mut te = terms_enum(&r)?;
        Assert::assert_true(te.seek_exact(&LuceneTestCase::new_bytes_ref_str("foo"))?);
        assert_equals(&179, &te.total_term_freq()?);
        Assert::assert_true(te.seek_exact(&LuceneTestCase::new_bytes_ref_str("bar"))?);
        assert_equals(&368, &te.total_term_freq()?);
    }

    close(Some(r), w, dir)
}

/// Port of `TestCustomTermFreq.testTotalTermFreqTermDoc`.
fn test_total_term_freq_term_doc() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = get_term_doc_field_type()?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar"],
        &[42, 128],
        &field_type,
    )?);
    w.add_document(&doc)?;

    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar"],
        &[50, 60],
        &field_type,
    )?);
    w.add_document(&doc)?;

    {
        let r = reader(&w)?;
        {
            let mut te = terms_enum(&r)?;
            Assert::assert_true(te.seek_exact(&LuceneTestCase::new_bytes_ref_str("foo"))?);
            assert_equals(&92, &te.total_term_freq()?);
            Assert::assert_true(te.seek_exact(&LuceneTestCase::new_bytes_ref_str("bar"))?);
            assert_equals(&188, &te.total_term_freq()?);
        }
        r.close()?;
    }
    close(None, w, dir)
}

/// you can't index proximity with custom term freqs:
///
/// Port of `TestCustomTermFreq.testInvalidProx`.
fn test_invalid_prox() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = FieldType::new_from(TextField::type_not_stored());
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar", "foo", "bar"],
        &[42, 128, 17, 100],
        &field_type,
    )?);
    let e = expect_throws(ISE, || w.add_document(&doc));
    assert_equals(
        &Some("field \"field\": cannot index positions while using custom TermFrequencyAttribute"),
        &e.message(),
    );
    close(None, w, dir)
}

/// you can't index DOCS_ONLY with custom term freq
///
/// Port of `TestCustomTermFreq.testInvalidDocsOnly`.
fn test_invalid_docs_only() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = FieldType::new_from(TextField::type_not_stored());
    field_type.set_index_options(IndexOptions::DOCS)?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar", "foo", "bar"],
        &[42, 128, 17, 100],
        &field_type,
    )?);
    let e = expect_throws(ISE, || w.add_document(&doc));
    assert_equals(
        &Some("field \"field\": must index term freq while using custom TermFrequencyAttribute"),
        &e.message(),
    );
    close(None, w, dir)
}

/// sum of term freqs must fit in an int
///
/// Port of `TestCustomTermFreq.testOverflowInt`.
fn test_overflow_int() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = FieldType::new_from(TextField::type_not_stored());
    field_type.set_index_options(IndexOptions::DOCS)?;

    let doc = doc_with(Field::new(
        "field",
        "this field should be indexed".to_string(),
        field_type.clone(),
    )?);
    w.add_document(&doc)?;

    let doc2 = doc_with(canned_field(
        "field",
        &["foo", "bar"],
        &[3, i32::MAX],
        &field_type,
    )?);
    expect_throws(IAE, || w.add_document(&doc2));

    let r = reader(&w)?;
    assert_equals(&1, &r.num_docs());

    close(Some(r), w, dir)
}

/// Port of `TestCustomTermFreq.testNoOverflowInt`.
fn test_no_overflow_int() -> Result<()> {
    let (dir, w) = open()?;

    // Using the termdoc field type enables us to store large term frequencies that would
    // otherwise overflow totalTermFreq
    let field_type = get_term_doc_field_type()?;

    let doc2 = doc_with(canned_field(
        "field",
        &["foo", "bar"],
        &[3, i32::MAX],
        &field_type,
    )?);
    w.add_document(&doc2)?;
    {
        let r = reader(&w)?;
        {
            assert_equals(&1, &r.num_docs());
            let mut te = terms_enum(&r)?;
            Assert::assert_true(te.seek_exact(&LuceneTestCase::new_bytes_ref_str("foo"))?);
            assert_equals(&3, &te.total_term_freq()?);

            // `termsEnum.postings(null)`: flags `PostingsEnum.FREQS`.
            let p = Some(te.postings(None, POSTINGS_ENUM_FREQS)?);
            assert_not_null(&p);
            assert_postings(&mut *p.expect("checked by assertNotNull"), &[(0, 3)])?;

            Assert::assert_true(te.seek_exact(&LuceneTestCase::new_bytes_ref_str("bar"))?);
            assert_equals(&(i32::MAX as i64), &te.total_term_freq()?);

            let p = Some(te.postings(None, POSTINGS_ENUM_FREQS)?);
            assert_not_null(&p);
            assert_postings(&mut *p.expect("checked by assertNotNull"), &[(0, i32::MAX)])?;
        }
        r.close()?;
    }
    close(None, w, dir)
}

/// Port of `TestCustomTermFreq.testInvalidTermVectorPositions`.
fn test_invalid_term_vector_positions() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = get_term_doc_field_type()?;
    field_type.set_store_term_vectors(true)?;
    field_type.set_store_term_vector_positions(true)?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar", "foo", "bar"],
        &[42, 128, 17, 100],
        &field_type,
    )?);
    let e = expect_throws(IAE, || w.add_document(&doc));
    assert_equals(
        &Some("field \"field\": cannot index term vector positions while using custom TermFrequencyAttribute"),
        &e.message(),
    );
    close(None, w, dir)
}

/// Port of `TestCustomTermFreq.testInvalidTermVectorOffsets`.
fn test_invalid_term_vector_offsets() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = get_term_doc_field_type()?;
    field_type.set_store_term_vectors(true)?;
    field_type.set_store_term_vector_offsets(true)?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar", "foo", "bar"],
        &[42, 128, 17, 100],
        &field_type,
    )?);
    let e = expect_throws(IAE, || w.add_document(&doc));
    assert_equals(
        &Some("field \"field\": cannot index term vector offsets while using custom TermFrequencyAttribute"),
        &e.message(),
    );
    close(None, w, dir)
}

/// The term-vector checks of `testTermVectors` for one document: `(term,
/// freq)` per term, in the order Java seeks them.
fn assert_term_vector(r: &Arc<dyn IndexReader>, doc: i32, expected: &[(&str, i32)]) -> Result<()> {
    let fields = r
        .term_vectors()?
        .get(doc)?
        .unwrap_or_else(|| crate::support::npe("termVectors().get(doc)"));
    let mut te = fields
        .terms("field")?
        .unwrap_or_else(|| crate::support::npe("fields.terms(\"field\")"))
        .iterator()?;
    for &(term, freq) in expected {
        Assert::assert_true(te.seek_exact(&LuceneTestCase::new_bytes_ref_str(term))?);
        assert_equals(&(freq as i64), &te.total_term_freq()?);
        // `termsEnum.postings(null)`: flags `PostingsEnum.FREQS`.
        let p = Some(te.postings(None, POSTINGS_ENUM_FREQS)?);
        assert_not_null(&p);
        assert_postings(&mut *p.expect("checked by assertNotNull"), &[(0, freq)])?;
    }
    Ok(())
}

/// Port of `TestCustomTermFreq.testTermVectors`.
fn test_term_vectors() -> Result<()> {
    let (dir, w) = open()?;

    let field_type = get_term_doc_field_type()?;
    field_type.set_store_term_vectors(true)?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar"],
        &[42, 128],
        &field_type,
    )?);
    w.add_document(&doc)?;

    field_type.set_index_options(IndexOptions::DOCS_AND_CUSTOM_FREQS)?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar"],
        &[50, 60],
        &field_type,
    )?);
    w.add_document(&doc)?;

    let r = reader(&w)?;

    assert_term_vector(&r, 0, &[("bar", 128), ("foo", 42)])?;
    assert_term_vector(&r, 1, &[("bar", 60), ("foo", 50)])?;

    close(Some(r), w, dir)
}

/// Similarity holds onto the FieldInvertState for subsequent verification.
///
/// `private static class NeverForgetsSimilarity extends Similarity`. Java
/// keeps a reference to the state object, which `IndexingChain` resets
/// only when it inverts the field again; the port keeps a copy of it, taken
/// in `computeNorm`, which holds the same values.
#[derive(Debug)]
struct NeverForgetsSimilarity {
    last_state: Mutex<Option<FieldInvertState>>,
}

/// `NeverForgetsSimilarity.INSTANCE`.
static INSTANCE: LazyLock<Arc<NeverForgetsSimilarity>> = LazyLock::new(|| {
    Arc::new(NeverForgetsSimilarity {
        last_state: Mutex::new(None),
    })
});

impl std::fmt::Display for NeverForgetsSimilarity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NeverForgetsSimilarity")
    }
}

impl Similarity for NeverForgetsSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestCustomTermFreq$NeverForgetsSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        *self.last_state.lock().unwrap_or_else(|e| e.into_inner()) = Some(state.clone());
        Ok(1)
    }

    fn scorer<'a>(
        &'a self,
        _boost: f32,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        Throwable::new("java.lang.UnsupportedOperationException", "").throw()
    }
}

/// Port of `TestCustomTermFreq.testFieldInvertState`.
fn test_field_invert_state() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_similarity(Some(Arc::clone(&*INSTANCE) as Arc<dyn Similarity>))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let field_type = get_term_doc_field_type()?;
    let doc = doc_with(canned_field(
        "field",
        &["foo", "bar"],
        &[42, 128],
        &field_type,
    )?);
    w.add_document(&doc)?;
    let fis = INSTANCE
        .last_state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| crate::support::npe("NeverForgetsSimilarity.INSTANCE.lastState"));
    assert_equals(&128, &fis.max_term_frequency());
    assert_equals(&2, &fis.unique_term_count());
    assert_equals(&0, &fis.num_overlap());
    assert_equals(&2, &fis.length());

    w.close()?;
    dir.close()
}
