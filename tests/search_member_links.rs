//! Links between `search` members that task #269 block 5 ported, each checked
//! where its Java caller makes it observable.
//!
//! Every test names the Lucene 10.5.0 source of the link it pins; the Java
//! citations are relative to `lucene/core/src/java/org/apache/lucene/`.

use std::cell::RefCell;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{
    Document, DoubleDocValuesField, IntPoint, KnnFloatVectorField, NumericDocValuesField, Store,
    StringField, TextField,
};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{
    directory_reader, DirectoryReader, ImpactsEnum, IndexReader, LeafReader, PostingsEnum,
    SeekStatus, Term, TermState, Terms, TermsEnum,
};
use rucene::search::conjunction_disi::ConjunctionMember;
use rucene::search::conjunction_utils::ConjunctionUtils;
use rucene::search::fuzzy_terms_enum::FuzzyTermsEnum;
use rucene::search::knn_float_vector_query::KnnFloatVectorQuery;
use rucene::search::multi_term_query_constant_score_wrapper::MultiTermQueryConstantScoreWrapper;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::{
    default_bulk, double_values_source, long_values_source, range, BoostQuery, ConstantScoreQuery,
    ConstantScoreScorer, DocAndScoreQuery, DocIdSetIterator, DoubleValues, DoubleValuesSource,
    Explanation, FieldExistsQuery, FuzzyQuery, IndexSearcher, MatchAllDocsQuery,
    PatienceKnnVectorQuery, PrefixQuery, Query, QueryVisitor, ScoreMode, SegmentCacheable,
    SharedVectorScorer, Sort, TermQuery, TwoPhaseIterator, VectorScorer, VectorScorerBulk,
    NO_MORE_DOCS,
};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::CompiledAutomaton;
use rucene::util::{BytesRef, IOBooleanSupplier};

fn ensure_codec() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
}

/// Writes `docs` into one segment of a fresh in-memory index.
fn index(docs: Vec<Document>) -> Arc<dyn DirectoryReader> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(NoMergePolicy::instance()))
        .expect("no merges");
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("index writer");
    for doc in &docs {
        writer.add_document(doc).expect("add document");
    }
    writer.commit().expect("commit");
    directory_reader::open(dir).expect("open the reader")
}

fn searcher(reader: &Arc<dyn DirectoryReader>) -> IndexSearcher {
    IndexSearcher::new(Arc::clone(reader) as Arc<dyn IndexReader>).expect("searcher")
}

fn drain(iterator: &mut dyn DocIdSetIterator) -> Vec<i32> {
    let mut docs = Vec::new();
    loop {
        let doc = iterator.next_doc().expect("next doc");
        if doc == NO_MORE_DOCS {
            return docs;
        }
        docs.push(doc);
    }
}

// ---------------------------------------------------------------------------
// AbstractMultiTermQueryConstantScoreWrapper.visit accepts the field through
// getField() (AbstractMultiTermQueryConstantScoreWrapper.java:86-95).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FieldRecorder {
    fields: RefCell<Vec<String>>,
}

impl QueryVisitor for FieldRecorder {
    fn accept_field(&self, field: &str) -> bool {
        self.fields.borrow_mut().push(field.to_string());
        true
    }
}

#[test]
fn the_constant_score_wrapper_offers_its_field_to_the_visitor() {
    let prefix = PrefixQuery::new(Term::from_text("body", "ab")).expect("prefix query");
    let wrapper = MultiTermQueryConstantScoreWrapper::new(Arc::new(prefix));
    let mut recorder = FieldRecorder::default();
    wrapper.visit(&mut recorder);
    let fields = recorder.fields.borrow();
    assert_eq!(
        fields.first().map(String::as_str),
        Some("body"),
        "the wrapper asks first, with its query's field: {fields:?}"
    );
}

// ---------------------------------------------------------------------------
// ConjunctionUtils delegates to ConjunctionDISI.addScorer/addIterator/
// addTwoPhaseIterator (ConjunctionUtils.java:20, 41, 83-99).
// ---------------------------------------------------------------------------

/// Matches the even documents of an approximation.
struct EvenDocs {
    approximation: Box<dyn DocIdSetIterator>,
}

impl fmt::Debug for EvenDocs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EvenDocs")
    }
}

impl TwoPhaseIterator for EvenDocs {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self.approximation.as_mut()
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self.approximation.as_ref()
    }

    fn matches(&mut self) -> Result<bool> {
        Ok(self.approximation.doc_id() % 2 == 0)
    }

    fn match_cost(&self) -> f32 {
        1.0
    }
}

#[test]
fn conjunction_utils_adds_iterators_through_conjunction_disi() {
    let mut members: Vec<ConjunctionMember> = Vec::new();
    ConjunctionUtils::add_iterator(Box::new(range(0, 10).unwrap()), &mut members);
    ConjunctionUtils::add_iterator(Box::new(range(5, 20).unwrap()), &mut members);
    let mut both = ConjunctionUtils::create_conjunction(members)
        .expect("two clauses")
        .into_doc_id_set_iterator();
    assert_eq!(drain(both.as_mut()), vec![5, 6, 7, 8, 9]);
}

#[test]
fn conjunction_utils_adds_two_phase_iterators_through_conjunction_disi() {
    let mut members: Vec<ConjunctionMember> = vec![ConjunctionMember::from_iterator(Box::new(
        range(3, 30).unwrap(),
    ))];
    ConjunctionUtils::add_two_phase_iterator(
        Box::new(EvenDocs {
            approximation: Box::new(range(0, 12).unwrap()),
        }),
        &mut members,
    );
    let mut evens = ConjunctionUtils::create_conjunction(members)
        .expect("two clauses")
        .into_doc_id_set_iterator();
    assert_eq!(drain(evens.as_mut()), vec![4, 6, 8, 10]);
}

#[test]
fn conjunction_utils_intersects_scorers_through_conjunction_disi() {
    let scorers: Vec<Box<dyn rucene::search::Scorer>> = vec![
        Box::new(ConstantScoreScorer::from_iterator(
            1.0,
            ScoreMode::COMPLETE,
            Box::new(range(0, 8).unwrap()),
        )),
        Box::new(ConstantScoreScorer::from_iterator(
            1.0,
            ScoreMode::COMPLETE,
            Box::new(range(6, 40).unwrap()),
        )),
    ];
    let mut scored = ConjunctionUtils::intersect_scorers(scorers)
        .expect("two scorers")
        .into_doc_id_set_iterator();
    assert_eq!(drain(scored.as_mut()), vec![6, 7]);
}

// ---------------------------------------------------------------------------
// VectorScorer.bulk intersects through ConjunctionUtils.createConjunction
// (VectorScorer.java:62-68).
// ---------------------------------------------------------------------------

/// Scores each document of a range with its doc ID.
struct RangeVectorScorer {
    iterator: Box<dyn DocIdSetIterator>,
}

impl VectorScorer for RangeVectorScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(self.iterator.doc_id() as f32)
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.iterator.as_mut()
    }

    fn bulk(
        self: Box<Self>,
        matching_docs: Option<Box<dyn DocIdSetIterator>>,
    ) -> Result<Box<dyn VectorScorerBulk>> {
        default_bulk(SharedVectorScorer::new(self), matching_docs)
    }
}

#[test]
fn the_default_bulk_scorer_scores_the_intersection_with_the_matching_docs() {
    let scorer = Box::new(RangeVectorScorer {
        iterator: Box::new(range(0, 20).unwrap()),
    });
    let mut bulk = scorer
        .bulk(Some(Box::new(range(15, 40).unwrap())))
        .expect("bulk scorer");
    let mut buffer = rucene::index::DocAndFloatFeatureBuffer::new();
    let max = bulk
        .next_docs_and_scores(NO_MORE_DOCS, None, &mut buffer)
        .expect("score");
    assert_eq!(&buffer.docs[..buffer.size], &[15, 16, 17, 18, 19]);
    assert_eq!(
        &buffer.features[..buffer.size],
        &[15.0, 16.0, 17.0, 18.0, 19.0]
    );
    assert_eq!(max, 19.0);
}

// ---------------------------------------------------------------------------
// ConstantScoreWeight subclasses score with the weight's score()
// (ConstantScoreWeight.java:39-41).
// ---------------------------------------------------------------------------

fn scored_docs() -> Arc<dyn DirectoryReader> {
    let docs = (0..4)
        .map(|i| {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", format!("{i}"), Store::NO).unwrap(),
            ));
            doc.add(Box::new(IntPoint::new("rank", &[i]).unwrap()));
            doc.add(Box::new(NumericDocValuesField::new("dv", i64::from(i))));
            doc
        })
        .collect();
    index(docs)
}

fn scores_of(searcher: &IndexSearcher, query: Arc<dyn Query>) -> Vec<f32> {
    searcher
        .search(query, 10)
        .expect("search")
        .score_docs
        .iter()
        .map(|hit| hit.score)
        .collect()
}

#[test]
fn constant_score_queries_score_with_the_weights_score() {
    let reader = scored_docs();
    let searcher = searcher(&reader);
    let boosted = |query: Arc<dyn Query>, boost: f32| -> Arc<dyn Query> {
        Arc::new(BoostQuery::new(query, boost).unwrap())
    };
    assert_eq!(
        scores_of(
            &searcher,
            boosted(Arc::new(MatchAllDocsQuery::instance()), 2.5)
        ),
        vec![2.5; 4]
    );
    assert_eq!(
        scores_of(
            &searcher,
            boosted(IntPoint::new_range_query("rank", &[1], &[2]).unwrap(), 3.0)
        ),
        vec![3.0; 2]
    );
    assert_eq!(
        scores_of(
            &searcher,
            boosted(Arc::new(FieldExistsQuery::new("dv")), 1.5)
        ),
        vec![1.5; 4]
    );
    assert_eq!(
        scores_of(
            &searcher,
            boosted(
                Arc::new(ConstantScoreQuery::new(Arc::new(TermQuery::new(
                    Term::from_text("id", "2")
                )))),
                4.0
            )
        ),
        vec![4.0]
    );
}

// ---------------------------------------------------------------------------
// DoubleValuesSortField / LongValuesSortField install
// new DoubleValuesComparatorSource(producer, missingValue) and
// new LongValuesComparatorSource(producer, missingValue).
// ---------------------------------------------------------------------------

fn values_docs() -> Arc<dyn DirectoryReader> {
    let mut docs = Vec::new();
    for value in [Some(5i64), None, Some(1)] {
        let mut doc = Document::new();
        if let Some(value) = value {
            doc.add(Box::new(DoubleDocValuesField::new("d", value as f64)));
            doc.add(Box::new(NumericDocValuesField::new("l", value)));
        }
        docs.push(doc);
    }
    index(docs)
}

fn sorted_docs(searcher: &IndexSearcher, sort: Sort) -> Vec<i32> {
    let manager = TopFieldCollectorManager::new(sort, 3, None, i32::MAX).expect("manager");
    searcher
        .search_with_manager(Arc::new(MatchAllDocsQuery::instance()), &manager)
        .expect("search")
        .score_docs
        .iter()
        .map(|hit| hit.score_doc.doc)
        .collect()
}

#[test]
fn a_double_values_sort_places_documents_without_a_value_at_the_missing_value() {
    let reader = values_docs();
    let searcher = searcher(&reader);
    let double = double_values_source::get_sort_field_with_missing(
        double_values_source::from_double_field("d"),
        false,
        100.0,
    )
    .unwrap();
    assert_eq!(
        sorted_docs(&searcher, Sort::new_fields(vec![double]).unwrap()),
        vec![2, 0, 1]
    );
}

#[test]
fn a_long_values_sort_places_documents_without_a_value_at_the_missing_value() {
    let reader = values_docs();
    let searcher = searcher(&reader);
    let long = long_values_source::get_sort_field_with_missing(
        long_values_source::from_long_field("l"),
        false,
        100,
    )
    .unwrap();
    assert_eq!(
        sorted_docs(&searcher, Sort::new_fields(vec![long]).unwrap()),
        vec![2, 0, 1]
    );
}

// ---------------------------------------------------------------------------
// DoubleValuesSource.explain reads the score through
// DoubleValuesSource.constant(...) (DoubleValuesSource.java:68-79).
// ---------------------------------------------------------------------------

/// Twice the score of the document.
#[derive(Debug)]
struct TwiceTheScore;

struct TwiceValues<'a> {
    scores: Box<dyn DoubleValues + 'a>,
}

impl DoubleValues for TwiceValues<'_> {
    fn double_value(&mut self) -> Result<f64> {
        Ok(2.0 * self.scores.double_value()?)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.scores.advance_exact(doc)
    }
}

impl SegmentCacheable for TwiceTheScore {
    fn is_cacheable(&self, _ctx: &rucene::index::LeafReaderContext) -> bool {
        true
    }
}

impl DoubleValuesSource for TwiceTheScore {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &rucene::index::LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        Ok(Box::new(TwiceValues {
            scores: scores.expect("the source needs scores"),
        }))
    }

    fn needs_scores(&self) -> bool {
        true
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        other.as_any().is::<TwiceTheScore>()
    }

    fn source_hash(&self) -> u64 {
        2
    }

    fn to_source_string(&self) -> String {
        "twice".to_string()
    }
}

#[test]
fn explain_feeds_the_score_explanation_through_a_constant_source() {
    let reader = values_docs();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    let explanation = Arc::new(TwiceTheScore)
        .explain(
            &leaves[0],
            0,
            &Explanation::matched(1.25f32, "score", Vec::new()),
        )
        .expect("explain");
    assert!(explanation.is_match());
    assert_eq!(explanation.value().double_value(), 2.5);
    assert_eq!(explanation.description(), "twice");
}

// ---------------------------------------------------------------------------
// FuzzyQuery.getTermsEnum builds the enum around getTerm()
// (FuzzyQuery.java:204-214).
// ---------------------------------------------------------------------------

#[test]
fn a_fuzzy_query_matches_the_terms_near_its_term() {
    let docs = ["abcd", "abce", "wxyz"]
        .iter()
        .map(|text| {
            let mut doc = Document::new();
            doc.add(Box::new(
                TextField::new("f", text.to_string(), Store::NO).unwrap(),
            ));
            doc
        })
        .collect();
    let reader = index(docs);
    let searcher = searcher(&reader);
    let query = FuzzyQuery::new(Term::from_text("f", "abcd")).unwrap();
    let mut docs: Vec<i32> = searcher
        .search(Arc::new(query), 10)
        .unwrap()
        .score_docs
        .iter()
        .map(|hit| hit.doc)
        .collect();
    docs.sort_unstable();
    assert_eq!(docs, vec![0, 1]);
}

// ---------------------------------------------------------------------------
// FuzzyTermsEnum overrides TermsEnum.prepareSeekExact to forward to the
// wrapped enum (FuzzyTermsEnum.java:330-332).
// ---------------------------------------------------------------------------

/// A field whose intersections record their `prepare_seek_exact` calls.
struct RecordingTerms {
    inner: Box<dyn Terms>,
    prepared: Arc<AtomicUsize>,
}

impl Terms for RecordingTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        self.inner.iterator()
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(RecordingTermsEnum {
            inner: self.inner.intersect(compiled, start_term)?,
            prepared: Arc::clone(&self.prepared),
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
}

/// Delegates everything and answers `prepare_seek_exact` with `None`, counting
/// the calls.
struct RecordingTermsEnum {
    inner: Box<dyn TermsEnum>,
    prepared: Arc<AtomicUsize>,
}

impl fmt::Display for RecordingTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Recording({})", self.inner)
    }
}

impl TermsEnum for RecordingTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }

    fn prepare_seek_exact<'a>(
        &'a mut self,
        _text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        self.prepared.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.inner.seek_ord(ord)
    }

    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.inner.seek_term_state(text, state)
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

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.inner.next()
    }
}

#[test]
fn a_fuzzy_terms_enum_prepares_seeks_on_the_enum_it_wraps() {
    let mut doc = Document::new();
    doc.add(Box::new(
        TextField::new("f", "abcd abce wxyz".to_string(), Store::NO).unwrap(),
    ));
    let reader = index(vec![doc]);
    let leaves = Arc::clone(&reader).leaves().unwrap();
    let leaf: Arc<dyn LeafReader> = leaves[0].leaf_reader();
    let prepared = Arc::new(AtomicUsize::new(0));
    let terms: Arc<dyn Terms> = Arc::new(RecordingTerms {
        inner: leaf.terms("f").unwrap().expect("the field is indexed"),
        prepared: Arc::clone(&prepared),
    });
    let mut fuzzy = FuzzyTermsEnum::new(terms, Term::from_text("f", "abcd"), 1, 0, true)
        .expect("fuzzy terms enum");
    let as_terms_enum: &mut dyn TermsEnum = &mut fuzzy;
    let supplier = as_terms_enum
        .prepare_seek_exact(&BytesRef::new(b"abce".to_vec()))
        .expect("prepare");
    assert!(
        supplier.is_none(),
        "the wrapped enum's answer comes back, not a supplier over seek_exact"
    );
    assert_eq!(prepared.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------
// LiveIndexWriterConfig: similarity = IndexSearcher.getDefaultSimilarity()
// (index/LiveIndexWriterConfig.java:129).
// ---------------------------------------------------------------------------

#[test]
fn a_new_writer_config_shares_the_searchers_default_similarity() {
    let config = IndexWriterConfig::new();
    assert!(Arc::ptr_eq(
        &config.similarity(),
        &IndexSearcher::get_default_similarity()
    ));
}

// ---------------------------------------------------------------------------
// HnswGraphSearcher.searchLevel calls
// results.getSearchStrategy().nextVectorsBlock() after each candidate
// (util/hnsw/HnswGraphSearcher.java:363-365), which advances the patience
// counter of PatienceKnnVectorQuery.
// ---------------------------------------------------------------------------

/// Deterministic vectors in eight dimensions.
fn vector(seed: u64) -> Vec<f32> {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..8)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) % 1000) as f32 / 1000.0
        })
        .collect()
}

/// The visited-node count of a kNN query, read from the `DocAndScoreQuery` it
/// rewrites to, as `DocAndScoreQuery.visited()`.
fn visited(searcher: &IndexSearcher, query: Arc<dyn Query>) -> i64 {
    let rewritten = searcher.rewrite(query).expect("rewrite");
    rewritten
        .as_any()
        .downcast_ref::<DocAndScoreQuery>()
        .expect("a kNN query rewrites to a DocAndScoreQuery")
        .visited()
}

fn hit_docs(searcher: &IndexSearcher, query: Arc<dyn Query>) -> Vec<i32> {
    searcher
        .search(query, 50)
        .expect("kNN search")
        .score_docs
        .iter()
        .map(|hit| hit.doc)
        .collect()
}

/// Expected values are Lucene 10.5.0's on the same 2000 vectors, k = 50,
/// saturation threshold 0.5, patience 2 (oracle `PatienceOracle.java`, one
/// segment): the plain query visits 383 nodes and returns 50 hits; the
/// patience query stops after 71 visits with 43 hits, in this order.
#[test]
fn a_patience_query_stops_once_the_queue_saturates() {
    let docs = (0..2000u64)
        .map(|i| {
            let mut doc = Document::new();
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("v", &vector(i)).unwrap(),
            ));
            doc
        })
        .collect();
    let reader = index(docs);
    let searcher = searcher(&reader);
    let target = vector(7_777_777);
    let plain = || -> Arc<dyn Query> {
        Arc::new(KnnFloatVectorQuery::new("v", target.clone(), 50).unwrap())
    };
    let patient = || -> Arc<dyn Query> {
        Arc::new(
            PatienceKnnVectorQuery::from_float_query(
                Arc::new(KnnFloatVectorQuery::new("v", target.clone(), 50).unwrap()),
                0.5,
                2,
            )
            .unwrap(),
        )
    };
    assert_eq!(visited(&searcher, plain()), 383);
    assert_eq!(hit_docs(&searcher, plain()).len(), 50);
    assert_eq!(visited(&searcher, patient()), 71);
    assert_eq!(
        hit_docs(&searcher, patient()),
        vec![
            1238, 1954, 1757, 315, 1077, 1255, 1334, 240, 298, 504, 614, 1794, 375, 276, 777, 1920,
            833, 120, 206, 1921, 173, 257, 855, 1569, 1867, 103, 581, 1946, 1272, 1, 72, 1811,
            1655, 53, 573, 300, 1884, 1585, 261, 1993, 574, 472, 282,
        ]
    );
}

// ---------------------------------------------------------------------------
// PhraseQuery(int, String, BytesRef...) builds its terms with toTerms, and the
// matcher asserts termNotInReader for a term the segment lacks
// (PhraseQuery.java:256-258, 526, 558-560).
// ---------------------------------------------------------------------------

#[test]
fn a_phrase_from_term_bytes_keeps_the_terms_in_order() {
    let phrase = PhraseQuery::with_slop_bytes(
        1,
        "f",
        &[BytesRef::new(b"b".to_vec()), BytesRef::new(b"a".to_vec())],
    )
    .unwrap();
    assert_eq!(
        phrase.get_terms(),
        &[Term::from_text("f", "b"), Term::from_text("f", "a")]
    );
    assert_eq!(phrase.get_positions(), &[0, 1]);
}

#[test]
fn a_phrase_with_a_term_the_segment_lacks_matches_nothing() {
    let mut doc = Document::new();
    doc.add(Box::new(
        TextField::new("f", "a b".to_string(), Store::NO).unwrap(),
    ));
    let reader = index(vec![doc]);
    let searcher = searcher(&reader);
    let present = PhraseQuery::from_strings("f", &["a", "b"]).unwrap();
    assert_eq!(
        searcher
            .search(Arc::new(present), 10)
            .unwrap()
            .score_docs
            .len(),
        1
    );
    let absent = PhraseQuery::from_strings("f", &["a", "c"]).unwrap();
    assert!(searcher
        .search(Arc::new(absent), 10)
        .unwrap()
        .score_docs
        .is_empty());
}
