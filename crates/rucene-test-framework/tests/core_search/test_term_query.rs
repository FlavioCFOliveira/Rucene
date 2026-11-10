//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTermQuery.java`.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::filter_reader::{FilterLeafReader, FilterTerms, FilterTermsEnum};
use rucene::index::index_reader::CacheHelper;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::terms::{SeekStatus, TermState, TermStates};
use rucene::index::{
    FieldInvertState, ImpactsEnum, IndexReader, LeafReader, MultiReader, PostingsEnum, Term, Terms,
    TermsEnum,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::similarities::{CollectionStatistics, SimScorer, Similarity, TermStatistics};
use rucene::search::term_query::TermQuery;
use rucene::search::weight::Weight;
use rucene::search::{Query, ScoreMode};
use rucene::util::attribute::AttributeSource;
use rucene::util::{BytesRef, IOBooleanSupplier};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::dummy_total_hit_count_collector::DummyTotalHitCountCollector;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, assert_null, class, random, test};

/// `TestTermQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTermQuery",
        vec![
            test("testEquals", test_equals),
            test(
                "testCreateWeightDoesNotSeekIfScoresAreNotNeeded",
                test_create_weight_does_not_seek_if_scores_are_not_needed,
            ),
            test("testQueryMatchesCount", test_query_matches_count),
            test("testGetTermStates", test_get_term_states),
            test(
                "testWithWithDifferentScoreModes",
                test_with_with_different_score_modes,
            ),
        ],
    )
}

/// `new Term(field, text)`.
fn t(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

/// Port of `TestTermQuery.testEquals`.
fn test_equals() -> Result<()> {
    QueryUtils::check_equal(
        &TermQuery::new(t("foo", "bar")),
        &TermQuery::new(t("foo", "bar")),
    );
    QueryUtils::check_unequal(
        &TermQuery::new(t("foo", "bar")),
        &TermQuery::new(t("foo", "baz")),
    );
    let multi_reader = Arc::new(MultiReader::new(Vec::new(), true)?);
    let result = (|| -> Result<()> {
        let context = (Arc::clone(&multi_reader) as Arc<dyn IndexReader>).get_context()?;
        let searcher = IndexSearcher::from_context(context, None)?;
        QueryUtils::check_equal(
            &TermQuery::new(t("foo", "bar")),
            &TermQuery::with_states(
                t("foo", "bar"),
                Arc::new(TermStates::build(&searcher, &t("foo", "bar"), true)?),
            ),
        );
        Ok(())
    })();
    result.and(multi_reader.close())
}

/// `newIndexWriterConfig().setMergePolicy(NoMergePolicy.INSTANCE)`, then
/// the three segments: one with the term, one without it, and one without
/// the field.
fn three_segment_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<RandomIndexWriter> {
    let config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    // segment that contains the term
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "foo",
        "bar".to_string(),
        Store::NO,
    )?));
    w.add_document(&doc)?;
    w.get_reader()?.close()?;
    // segment that does not contain the term
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "foo",
        "baz".to_string(),
        Store::NO,
    )?));
    w.add_document(&doc)?;
    w.get_reader()?.close()?;
    // segment that does not contain the field
    w.add_document(&Document::new())?;
    Ok(w)
}

/// Port of `TestTermQuery.testCreateWeightDoesNotSeekIfScoresAreNotNeeded`.
fn test_create_weight_does_not_seek_if_scores_are_not_needed() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = three_segment_writer(&dir)?;

    let reader = w.get_reader()?;
    let no_seek_reader = no_seek_directory_reader(Arc::clone(&reader))?;
    let no_seek_searcher = IndexSearcher::new(Arc::clone(&no_seek_reader) as Arc<dyn IndexReader>)?;
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(t("foo", "bar")));
    let e = expect_throws("java.lang.AssertionError", || {
        no_seek_searcher.create_weight(
            no_seek_searcher.rewrite(Arc::clone(&query))?,
            ScoreMode::COMPLETE,
            1.0,
        )
    });
    assert_equals(&Some("no seek"), &e.message());

    no_seek_searcher.create_weight(
        no_seek_searcher.rewrite(Arc::clone(&query))?,
        ScoreMode::COMPLETE_NO_SCORES,
        1.0,
    )?; // no exception
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    // use a collector rather than searcher.count() which would just read the
    // doc freq instead of creating a scorer
    let total_hits = searcher.search_with_manager(
        Arc::clone(&query),
        &DummyTotalHitCountCollector::create_manager(),
    )?;
    assert_equals(&1, &total_hits);
    let query_with_context: Arc<dyn Query> = Arc::new(TermQuery::with_states(
        t("foo", "bar"),
        Arc::new(TermStates::build(&searcher, &t("foo", "bar"), true)?),
    ));
    let total_hits = searcher.search_with_manager(
        query_with_context,
        &DummyTotalHitCountCollector::create_manager(),
    )?;
    assert_equals(&1, &total_hits);

    drop(searcher);
    drop(no_seek_searcher);
    drop(no_seek_reader);
    // `IOUtils.close(reader, w, dir)`.
    let r1 = reader.close();
    let r2 = w.close();
    let r3 = dir.close();
    r1.and(r2).and(r3)
}

/// LUCENE-9620 Add Weight#count(LeafReaderContext)
///
/// Port of `TestTermQuery.testQueryMatchesCount`.
fn test_query_matches_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let random_num_docs = TestUtil::next_int(&*random(), 10, 100);
    let mut num_matching_docs = 0;

    for _i in 0..random_num_docs {
        let mut doc = Document::new();
        if random().next_boolean() {
            doc.add(Box::new(StringField::new(
                "foo",
                "bar".to_string(),
                Store::NO,
            )?));
            num_matching_docs += 1;
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;

    let reader = w.get_reader()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    let test_query: Arc<dyn Query> = Arc::new(TermQuery::new(t("foo", "bar")));
    assert_equals(
        &searcher.count(Arc::clone(&test_query))?,
        &num_matching_docs,
    );
    let weight = searcher.create_weight(test_query, ScoreMode::COMPLETE, 1.0)?;
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    assert_equals(&weight.count(&leaves[0])?, &num_matching_docs);

    drop(searcher);
    let r1 = reader.close();
    let r2 = w.close();
    let r3 = dir.close();
    r1.and(r2).and(r3)
}

/// Port of `TestTermQuery.testGetTermStates`.
fn test_get_term_states() -> Result<()> {
    // no term states:
    assert_null(
        &TermQuery::new(t("foo", "bar"))
            .get_term_states()
            .map(|_| "TermStates"),
    );

    let mut dir = LuceneTestCase::new_directory();
    let w = three_segment_writer(&dir)?;

    let reader = w.get_reader()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let query_with_context = TermQuery::with_states(
        t("foo", "bar"),
        Arc::new(TermStates::build(&searcher, &t("foo", "bar"), true)?),
    );
    assert_not_null(&query_with_context.get_term_states());
    drop(searcher);
    let r1 = reader.close();
    let r2 = w.close();
    let r3 = dir.close();
    r1.and(r2).and(r3)
}

/// `ScoreMode.values()`.
const SCORE_MODES: [ScoreMode; 5] = [
    ScoreMode::COMPLETE,
    ScoreMode::COMPLETE_NO_SCORES,
    ScoreMode::TOP_SCORES,
    ScoreMode::TOP_DOCS,
    ScoreMode::TOP_DOCS_WITH_SCORES,
];

/// The anonymous `Similarity` wrapping the existing similarity for testing.
#[derive(Debug)]
struct WrappingSimilarity {
    existing_similarity: Arc<dyn Similarity>,
    scorer_called: Arc<AtomicBool>,
}

impl Similarity for WrappingSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestTermQuery$1"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        self.existing_similarity.compute_norm(state)
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        self.scorer_called.store(true, Ordering::SeqCst);
        self.existing_similarity
            .scorer(boost, collection_stats, term_stats)
    }

    /// The owned form of `scorer`, which the weights call: it records the
    /// call as `scorer` does.
    fn scorer_owned(
        self: Arc<Self>,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + Send + Sync + 'static>
    where
        Self: 'static,
    {
        self.scorer_called.store(true, Ordering::SeqCst);
        Arc::clone(&self.existing_similarity).scorer_owned(boost, collection_stats, term_stats)
    }
}

/// The anonymous `TermQuery` subclass whose `createWeight` records the score
/// mode.
#[derive(Debug, Clone)]
struct RecordingTermQuery {
    inner: TermQuery,
    score_mode_in_weight: Arc<Mutex<Option<ScoreMode>>>,
}

impl Query for RecordingTermQuery {
    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        *self.score_mode_in_weight.lock().expect("scoreModeInWeight") = Some(score_mode);
        self.inner.create_weight(searcher, score_mode, boost)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.inner.visit(visitor)
    }

    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_query_string(field)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<RecordingTermQuery>()
                .is_some_and(|o| self.inner.query_eq(&o.inner))
    }

    fn query_hash(&self) -> u64 {
        self.inner.query_hash()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Port of `TestTermQuery.testWithWithDifferentScoreModes`.
fn test_with_with_different_score_modes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    // segment that contains the term
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "foo",
        "bar".to_string(),
        Store::NO,
    )?));
    w.add_document(&doc)?;
    w.get_reader()?.close()?;
    let reader = w.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let existing_similarity = Arc::clone(searcher.get_similarity());

    for score_mode in SCORE_MODES {
        let score_mode_in_weight: Arc<Mutex<Option<ScoreMode>>> = Arc::new(Mutex::new(None));
        let scorer_called = Arc::new(AtomicBool::new(false));
        searcher.set_similarity(Arc::new(WrappingSimilarity {
            // Wrapping existing similarity for testing
            existing_similarity: Arc::clone(&existing_similarity),
            scorer_called: Arc::clone(&scorer_called),
        }));
        let term_query = RecordingTermQuery {
            inner: TermQuery::new(t("foo", "bar")),
            score_mode_in_weight: Arc::clone(&score_mode_in_weight),
        };
        term_query.create_weight(&searcher, score_mode, 1f32)?;
        assert_equals(
            &Some(score_mode),
            &*score_mode_in_weight.lock().expect("scoreModeInWeight"),
        );
        assert_equals(
            &score_mode.needs_scores(),
            &scorer_called.load(Ordering::SeqCst),
        );
    }
    drop(searcher);
    let r1 = reader.close();
    let r2 = w.close();
    let r3 = dir.close();
    r1.and(r2).and(r3)
}

// -----------------------------------------------------------------------------
// NoSeekDirectoryReader
// -----------------------------------------------------------------------------

/// `new NoSeekDirectoryReader(in)`: `private static class
/// NoSeekDirectoryReader extends FilterDirectoryReader`, whose
/// `getReaderCacheHelper()` is `in.getReaderCacheHelper()`.
fn no_seek_directory_reader(r#in: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
    Ok(Arc::new(FilterDirectoryReader::new(
        r#in,
        Arc::new(NoSeekSubReaderWrapper),
        Arc::new(NoSeekDirectoryReaderWrapper),
    )?))
}

/// The anonymous `SubReaderWrapper` of `NoSeekDirectoryReader`.
struct NoSeekSubReaderWrapper;

impl SubReaderWrapper for NoSeekSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(NoSeekLeafReader {
            inner: FilterLeafReader::new(reader)?,
        }))
    }
}

/// `NoSeekDirectoryReader.doWrapDirectoryReader`.
struct NoSeekDirectoryReaderWrapper;

impl DirectoryReaderWrapper for NoSeekDirectoryReaderWrapper {
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        no_seek_directory_reader(inner)
    }

    fn subclass_simple_name(&self) -> &'static str {
        "NoSeekDirectoryReader"
    }
}

/// `throw new AssertionError("no seek")`.
fn no_seek() -> LuceneError {
    LuceneError::AssertionError("no seek".to_string())
}

/// The anonymous `FilterTerms` of `NoSeekLeafReader.terms`.
struct NoSeekTerms {
    inner: FilterTerms,
}

impl Terms for NoSeekTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(NoSeekTermsEnum {
            inner: FilterTermsEnum::new(self.inner.iterator()?),
        }))
    }

    fn size(&self) -> Result<i64> {
        self.inner.size()
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        self.inner.sum_total_term_freq()
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        self.inner.sum_doc_freq()
    }

    fn doc_count(&self) -> Result<i32> {
        self.inner.doc_count()
    }

    fn has_freqs(&self) -> bool {
        self.inner.has_freqs()
    }

    fn has_offsets(&self) -> bool {
        self.inner.has_offsets()
    }

    fn has_positions(&self) -> bool {
        self.inner.has_positions()
    }

    fn has_payloads(&self) -> bool {
        self.inner.has_payloads()
    }

    fn stats(&self) -> Result<String> {
        self.inner.stats()
    }
}

/// The anonymous `FilterTermsEnum` of `NoSeekTerms.iterator`, whose seeks
/// throw.
struct NoSeekTermsEnum {
    inner: FilterTermsEnum,
}

impl fmt::Display for NoSeekTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl TermsEnum for NoSeekTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Err(no_seek())
    }

    fn prepare_seek_exact<'a>(
        &'a mut self,
        _text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        Err(no_seek())
    }

    fn seek_term_state(&mut self, _term: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Err(no_seek())
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        Err(no_seek())
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(no_seek())
    }

    fn term(&self) -> Result<BytesRef> {
        self.inner.term()
    }

    fn ord(&self) -> Result<i64> {
        self.inner.ord()
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.inner.doc_freq()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.inner.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.inner.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.inner.impacts(flags)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.inner.term_state()
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.inner.next()
    }
}

/// `private static class NoSeekLeafReader extends FilterLeafReader`, whose
/// cache helpers are `in`'s.
struct NoSeekLeafReader {
    inner: FilterLeafReader,
}

impl fmt::Debug for NoSeekLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NoSeekLeafReader").finish_non_exhaustive()
    }
}

impl fmt::Display for NoSeekLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for NoSeekLeafReader {
    fn core(&self) -> &rucene::index::IndexReaderCore {
        LeafReader::core(&self.inner)
    }
    fn filter_leaf_reader_delegate(&self) -> Option<&Arc<dyn LeafReader>> {
        LeafReader::filter_leaf_reader_delegate(&self.inner)
    }
    fn term_vectors(&self) -> Result<Box<dyn rucene::index::TermVectors>> {
        LeafReader::term_vectors(&self.inner)
    }
    fn num_docs(&self) -> i32 {
        LeafReader::num_docs(&self.inner)
    }
    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }
    fn stored_fields(&self) -> Result<Box<dyn rucene::index::StoredFields>> {
        LeafReader::stored_fields(&self.inner)
    }
    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(&self.inner)
    }
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_core_cache_helper()
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_reader_cache_helper()
    }
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let terms = self.inner.terms(field)?;
        Ok(terms.map(|terms| {
            Box::new(NoSeekTerms {
                inner: FilterTerms::new(terms),
            }) as Box<dyn Terms>
        }))
    }
    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_numeric_doc_values(field)
    }
    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
        self.inner.get_binary_doc_values(field)
    }
    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
        self.inner.get_sorted_doc_values(field)
    }
    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
        self.inner.get_sorted_numeric_doc_values(field)
    }
    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
        self.inner.get_sorted_set_doc_values(field)
    }
    fn get_norm_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_norm_values(field)
    }
    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }
    fn get_float_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
        self.inner.get_float_vector_values(field)
    }
    fn get_byte_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
        self.inner.get_byte_vector_values(field)
    }
    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }
    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }
    fn get_field_infos(&self) -> rucene::index::FieldInfos {
        self.inner.get_field_infos()
    }
    fn get_live_docs(&self) -> Option<Box<dyn rucene::util::Bits>> {
        LeafReader::get_live_docs(&self.inner)
    }
    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn rucene::index::PointValues>>> {
        self.inner.get_point_values(field)
    }
    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }
    fn get_meta_data(&self) -> rucene::index::LeafMetaData {
        self.inner.get_meta_data()
    }
}
