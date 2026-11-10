//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSynonymQuery.java`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::term_state::Impact;
use rucene::document::{Document, Field, FieldType, Store, StringField, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{
    FreqAndNormBuffer, Impacts, ImpactsEnum, ImpactsSource, IndexReader, MultiReader, PostingsEnum,
    Term,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_matcher::SharedPostings;
use rucene::search::synonym_query::{merge_impacts, Builder as SynonymQueryBuilder, SynonymQuery};
use rucene::search::term_query::TermQuery;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};
use rucene::search::{
    query_to_string, BooleanQueryBuilder, DocIdSetIterator, MatchNoDocsQuery, Occur, Query,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::assert_query_equals;
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_not_equals, class, expect_throws_panic, random,
    test, IAE,
};

/// `TestSynonymQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSynonymQuery",
        vec![
            test("testEquals", test_equals),
            test("testHashCode", test_hash_code),
            test("testGetField", test_get_field),
            test("testBogusParams", test_bogus_params),
            test("testToString", test_to_string),
            test("testScores", test_scores),
            test("testBoosts", test_boosts),
            test("testMergeImpacts", test_merge_impacts),
            test("testRandomTopDocs", test_random_top_docs),
            test("testRewrite", test_rewrite),
        ],
    )
}

/// `new SynonymQuery.Builder(field)`.
fn builder(field: &str) -> SynonymQueryBuilder {
    SynonymQueryBuilder::new(Some(field))
}

/// `new Term(field, text)`.
fn t(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

/// Port of `TestSynonymQuery.testEquals`.
fn test_equals() -> Result<()> {
    QueryUtils::check_equal(&builder("foo").build(), &builder("foo").build());
    QueryUtils::check_equal(
        &builder("foo").add_term(&t("foo", "bar"))?.build(),
        &builder("foo").add_term(&t("foo", "bar"))?.build(),
    );

    QueryUtils::check_equal(
        &builder("a")
            .add_term(&t("a", "a"))?
            .add_term(&t("a", "b"))?
            .build(),
        &builder("a")
            .add_term(&t("a", "b"))?
            .add_term(&t("a", "a"))?
            .build(),
    );

    QueryUtils::check_equal(
        &builder("field")
            .add_term_with_boost(&t("field", "b"), 0.4f32)?
            .add_term_with_boost(&t("field", "c"), 0.2f32)?
            .add_term(&t("field", "d"))?
            .build(),
        &builder("field")
            .add_term_with_boost(&t("field", "b"), 0.4f32)?
            .add_term_with_boost(&t("field", "c"), 0.2f32)?
            .add_term(&t("field", "d"))?
            .build(),
    );

    QueryUtils::check_unequal(
        &builder("field")
            .add_term_with_boost(&t("field", "a"), 0.4f32)?
            .build(),
        &builder("field")
            .add_term_with_boost(&t("field", "b"), 0.4f32)?
            .build(),
    );

    QueryUtils::check_unequal(
        &builder("field")
            .add_term_with_boost(&t("field", "a"), 0.2f32)?
            .build(),
        &builder("field")
            .add_term_with_boost(&t("field", "a"), 0.4f32)?
            .build(),
    );

    QueryUtils::check_unequal(
        &builder("field1")
            .add_term_with_boost(&t("field1", "b"), 0.4f32)?
            .build(),
        &builder("field2")
            .add_term_with_boost(&t("field2", "b"), 0.4f32)?
            .build(),
    );
    Ok(())
}

/// Port of `TestSynonymQuery.testHashCode`.
fn test_hash_code() -> Result<()> {
    let q0 = builder("field1")
        .add_term_with_boost(&t("field1", "a"), 0.4f32)?
        .build();
    let q1 = builder("field1")
        .add_term_with_boost(&t("field1", "a"), 0.4f32)?
        .build();
    let q2 = builder("field2")
        .add_term_with_boost(&t("field2", "a"), 0.4f32)?
        .build();

    assert_equals(&q0.query_hash(), &q1.query_hash());
    assert_not_equals(&q0.query_hash(), &q2.query_hash());
    Ok(())
}

/// Port of `TestSynonymQuery.testGetField`.
fn test_get_field() -> Result<()> {
    let query = builder("field1").add_term(&t("field1", "a"))?.build();
    assert_equals("field1", query.get_field());
    Ok(())
}

/// Port of `TestSynonymQuery.testBogusParams`.
fn test_bogus_params() -> Result<()> {
    expect_throws(IAE, || {
        builder("field1")
            .add_term(&t("field1", "a"))?
            .add_term(&t("field2", "b"))
            .map(|_| ())
    });

    expect_throws(IAE, || {
        builder("field1")
            .add_term_with_boost(&t("field1", "a"), 1.3f32)
            .map(|_| ())
    });

    expect_throws(IAE, || {
        builder("field1")
            .add_term_with_boost(&t("field1", "a"), f32::NAN)
            .map(|_| ())
    });

    expect_throws(IAE, || {
        builder("field1")
            .add_term_with_boost(&t("field1", "a"), f32::INFINITY)
            .map(|_| ())
    });

    expect_throws(IAE, || {
        builder("field1")
            .add_term_with_boost(&t("field1", "a"), f32::NEG_INFINITY)
            .map(|_| ())
    });

    expect_throws(IAE, || {
        builder("field1")
            .add_term_with_boost(&t("field1", "a"), -0.3f32)
            .map(|_| ())
    });

    expect_throws(IAE, || {
        builder("field1")
            .add_term_with_boost(&t("field1", "a"), 0f32)
            .map(|_| ())
    });

    expect_throws(IAE, || {
        builder("field1")
            .add_term_with_boost(&t("field1", "a"), -0f32)
            .map(|_| ())
    });

    expect_throws_panic("java.lang.NullPointerException", || {
        let _ = SynonymQueryBuilder::new(None).add_term_with_boost(&t("field1", "a"), -0f32);
    });

    expect_throws_panic("java.lang.NullPointerException", || {
        SynonymQueryBuilder::new(None).build();
    });
    Ok(())
}

/// Port of `TestSynonymQuery.testToString`.
fn test_to_string() -> Result<()> {
    assert_equals("Synonym()", &query_to_string(&builder("foo").build()));
    let t1 = t("foo", "bar");
    assert_equals(
        "Synonym(foo:bar)",
        &query_to_string(&builder("foo").add_term(&t1)?.build()),
    );
    let t2 = t("foo", "baz");
    assert_equals(
        "Synonym(foo:bar foo:baz)",
        &query_to_string(&builder("foo").add_term(&t1)?.add_term(&t2)?.build()),
    );
    Ok(())
}

/// Port of `TestSynonymQuery.testScores`.
fn test_scores() -> Result<()> {
    do_test_scores(1)?;
    do_test_scores(i32::MAX)
}

/// `private void doTestScores(int totalHitsThreshold)`.
fn do_test_scores(total_hits_threshold: i32) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("f", "a".to_string(), Store::NO)?));
    w.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("f", "b".to_string(), Store::NO)?));
    for _i in 0..10 {
        w.add_document(&doc)?;
    }
    let boost = if random().next_boolean() {
        random().next_float()
    } else {
        1f32
    };
    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query: Arc<dyn Query> = Arc::new(
        builder("f")
            .add_term_with_boost(&t("f", "a"), if boost == 0.0 { 1f32 } else { boost })?
            .add_term_with_boost(&t("f", "b"), if boost == 0.0 { 1f32 } else { boost })?
            .build(),
    );

    let collector_manager = TopScoreDocCollectorManager::with_num_hits(
        reader.num_docs().min(total_hits_threshold),
        total_hits_threshold,
    )?;
    let top_docs = searcher.search_with_manager(query, &collector_manager)?;
    if top_docs.total_hits.value() < i64::from(total_hits_threshold) {
        assert_equals(
            &TotalHits::new(11, TotalHitsRelation::EQUAL_TO)?,
            &top_docs.total_hits,
        );
    }
    // All docs must have the same score
    for i in 0..top_docs.score_docs.len() {
        assert_equals_delta(
            f64::from(top_docs.score_docs[0].score),
            f64::from(top_docs.score_docs[i].score),
            0.0,
        );
    }

    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()
}

/// Port of `TestSynonymQuery.testBoosts`.
fn test_boosts() -> Result<()> {
    do_test_boosts(1)?;
    do_test_boosts(i32::MAX)
}

/// `public void doTestBoosts(int totalHitsThreshold)`.
fn do_test_boosts(total_hits_threshold: i32) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut doc = Document::new();
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_omit_norms(true)?;
    let field = |value: &str| -> Result<Box<Field>> {
        Ok(Box::new(Field::new("f", value.to_string(), ft.clone())?))
    };
    doc.add(field("c")?);
    w.add_document(&doc)?;
    for i in 0..10 {
        doc.clear();
        doc.add(field("a a a a")?);
        w.add_document(&doc)?;
        if i % 2 == 0 {
            doc.clear();
            doc.add(field("b b")?);
            w.add_document(&doc)?;
        } else {
            doc.clear();
            doc.add(field("a a b")?);
            w.add_document(&doc)?;
        }
    }
    doc.clear();
    doc.add(field("c")?);
    w.add_document(&doc)?;
    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query: Arc<dyn Query> = Arc::new(
        builder("f")
            .add_term_with_boost(&t("f", "a"), 0.25f32)?
            .add_term_with_boost(&t("f", "b"), 0.5f32)?
            .add_term(&t("f", "c"))?
            .build(),
    );

    let collector_manager = TopScoreDocCollectorManager::with_num_hits(
        reader.num_docs().min(total_hits_threshold),
        total_hits_threshold,
    )?;
    let top_docs = searcher.search_with_manager(query, &collector_manager)?;
    if top_docs.total_hits.value() < i64::from(total_hits_threshold) {
        assert_equals(
            &TotalHitsRelation::EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
        assert_equals(&22, &top_docs.total_hits.value());
    } else {
        assert_equals(
            &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
            &top_docs.total_hits.relation(),
        );
    }
    // All docs must have the same score
    for i in 0..top_docs.score_docs.len() {
        assert_equals_delta(
            f64::from(top_docs.score_docs[0].score),
            f64::from(top_docs.score_docs[i].score),
            0.0,
        );
    }

    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()
}

/// `new Impact(freq, norm)`.
fn imp(freq: i32, norm: i64) -> Impact {
    Impact::new(freq, norm)
}

/// Port of `TestSynonymQuery.testMergeImpacts`.
fn test_merge_impacts() -> Result<()> {
    let impacts1 = DummyImpactsEnum::new();
    impacts1.reset(
        42,
        vec![
            vec![imp(3, 10), imp(5, 12), imp(8, 13)],
            vec![imp(5, 11), imp(8, 13), imp(12, 14)],
        ],
        vec![110, 945],
    );
    let impacts2 = DummyImpactsEnum::new();
    impacts2.reset(
        45,
        vec![
            vec![imp(2, 10), imp(6, 13)],
            vec![imp(3, 9), imp(5, 11), imp(7, 13)],
        ],
        vec![90, 1000],
    );

    let mut merged_impacts = merge_impacts(
        vec![
            SharedPostings::new(Box::new(impacts1.clone())),
            SharedPostings::new(Box::new(impacts2.clone())),
        ],
        vec![1f32, 1f32],
    );
    assert_impacts(
        &[
            vec![imp(5, 10), imp(7, 12), imp(14, 13)],
            vec![imp(i32::MAX, 1)],
        ],
        &[90, 1000],
        &*merged_impacts.get_impacts()?,
    );

    let mut merged_boosted_impacts = merge_impacts(
        vec![
            SharedPostings::new(Box::new(impacts1.clone())),
            SharedPostings::new(Box::new(impacts2.clone())),
        ],
        vec![0.3f32, 0.9f32],
    );
    assert_impacts(
        &[
            vec![imp(3, 10), imp(4, 12), imp(9, 13)],
            vec![imp(i32::MAX, 1)],
        ],
        &[90, 1000],
        &*merged_boosted_impacts.get_impacts()?,
    );

    // docID is > the first doIdUpTo of impacts1
    impacts2.reset(
        112,
        vec![
            vec![imp(2, 10), imp(6, 13)],
            vec![imp(3, 9), imp(5, 11), imp(7, 13)],
        ],
        vec![150, 1000],
    );
    assert_impacts(
        &[
            vec![imp(3, 10), imp(5, 12), imp(8, 13)], // same as impacts1
            vec![imp(3, 9), imp(10, 11), imp(15, 13), imp(19, 14)],
        ],
        &[110, 945],
        &*merged_impacts.get_impacts()?,
    );

    assert_impacts(
        &[
            vec![imp(1, 10), imp(2, 12), imp(3, 13)], // same as impacts1*boost
            vec![imp(3, 9), imp(7, 11), imp(10, 13), imp(11, 14)],
        ],
        &[110, 945],
        &*merged_boosted_impacts.get_impacts()?,
    );
    Ok(())
}

/// `private static void assertEquals(Impact[][] impacts, int[] docIdUpTo, Impacts actual)`.
fn assert_impacts(impacts: &[Vec<Impact>], doc_id_up_to: &[i32], actual: &dyn Impacts) {
    assert_equals(&(impacts.len() as i32), &actual.num_levels());
    for i in 0..impacts.len() {
        assert_equals(&doc_id_up_to[i], &actual.doc_id_up_to(i as i32));
        assert_equals(&impacts[i], &copy_of(&actual.get_impacts(i as i32)));
    }
}

/// `private static List<Impact> copyOf(FreqAndNormBuffer buffer)`.
fn copy_of(buffer: &FreqAndNormBuffer) -> Vec<Impact> {
    let mut copy = Vec::new();
    for i in 0..buffer.size {
        copy.push(Impact::new(buffer.freqs[i], buffer.norms[i]));
    }
    copy
}

/// The `docID`, `impacts` and `docIdUpTo` fields of `DummyImpactsEnum`,
/// which `reset` changes after the enum was handed to `mergeImpacts`.
#[derive(Debug, Default)]
struct DummyState {
    doc_id: i32,
    impacts: Vec<Vec<Impact>>,
    doc_id_up_to: Vec<i32>,
}

/// `private static class DummyImpactsEnum extends ImpactsEnum`.
///
/// The test keeps a reference to each enum it gives `mergeImpacts`, to reset
/// it; the clones share the state.
#[derive(Debug, Clone)]
struct DummyImpactsEnum {
    state: Rc<RefCell<DummyState>>,
}

impl DummyImpactsEnum {
    fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(DummyState::default())),
        }
    }

    fn reset(&self, doc_id: i32, impacts: Vec<Vec<Impact>>, doc_id_up_to: Vec<i32>) {
        let mut state = self.state.borrow_mut();
        state.doc_id = doc_id;
        state.impacts = impacts;
        state.doc_id_up_to = doc_id_up_to;
    }
}

fn uoe() -> LuceneError {
    LuceneError::UnsupportedOperation(String::new())
}

fn throw_uoe() -> ! {
    randomizedtesting::jdk::exception::throw_without_message(
        "java.lang.UnsupportedOperationException",
    )
}

impl std::fmt::Display for DummyImpactsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::core_search::support::object_to_string(
            "org.apache.lucene.search.TestSynonymQuery$DummyImpactsEnum",
            self,
        ))
    }
}

impl DocIdSetIterator for DummyImpactsEnum {
    fn doc_id(&self) -> i32 {
        self.state.borrow().doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        Err(uoe())
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(uoe())
    }

    fn cost(&self) -> i64 {
        throw_uoe()
    }
}

impl PostingsEnum for DummyImpactsEnum {
    fn freq(&self) -> Result<i32> {
        Err(uoe())
    }

    fn next_position(&mut self) -> Result<i32> {
        Err(uoe())
    }

    fn start_offset(&self) -> i32 {
        throw_uoe()
    }

    fn end_offset(&self) -> i32 {
        throw_uoe()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Err(uoe())
    }
}

impl ImpactsSource for DummyImpactsEnum {
    fn advance_shallow(&mut self, _target: i32) -> Result<()> {
        Err(uoe())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        // The anonymous `Impacts` reads the enum's fields when called.
        Ok(Box::new(DummyImpacts {
            state: Rc::clone(&self.state),
        }))
    }
}

impl ImpactsEnum for DummyImpactsEnum {}

/// The anonymous `Impacts` of `DummyImpactsEnum.getImpacts()`.
struct DummyImpacts {
    state: Rc<RefCell<DummyState>>,
}

impl Impacts for DummyImpacts {
    fn num_levels(&self) -> i32 {
        self.state.borrow().impacts.len() as i32
    }

    fn doc_id_up_to(&self, level: i32) -> i32 {
        self.state.borrow().doc_id_up_to[level as usize]
    }

    fn get_impacts(&self, level: i32) -> FreqAndNormBuffer {
        let mut buffer = FreqAndNormBuffer::new();
        self.state.borrow().impacts[level as usize]
            .iter()
            .for_each(|impact| buffer.add(impact.freq, impact.norm));
        buffer
    }
}

/// Port of `TestSynonymQuery.testRandomTopDocs`.
fn test_random_top_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(128 * 8 * 8 * 3)
    } else {
        LuceneTestCase::at_least(100)
    }; // at night, make sure some terms have skip data
    for _i in 0..num_docs {
        let mut doc = Document::new();
        let num_values = random().next_int_bound(1 << random().next_int_bound(5));
        let start = random().next_int_bound(10);
        for j in 0..num_values {
            let freq = TestUtil::next_int(&*random(), 1, 1 << random().next_int_bound(3));
            for _k in 0..freq {
                doc.add(Box::new(TextField::new(
                    "foo",
                    (start + j).to_string(),
                    Store::NO,
                )?));
            }
        }
        w.add_document(&doc)?;
    }
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    for term1 in 0..15 {
        let mut term2;
        loop {
            term2 = random().next_int_bound(15);
            if term1 != term2 {
                break;
            }
        }
        let boost1 = if random().next_boolean() {
            random().next_float().max(f32::MIN_POSITIVE)
        } else {
            1f32
        };
        let boost2 = if random().next_boolean() {
            random().next_float().max(f32::MIN_POSITIVE)
        } else {
            1f32
        };
        let query: Arc<dyn Query> = Arc::new(
            builder("foo")
                .add_term_with_boost(&t("foo", &term1.to_string()), boost1)?
                .add_term_with_boost(&t("foo", &term2.to_string()), boost2)?
                .build(),
        );

        let complete_manager = TopScoreDocCollectorManager::with_num_hits(10, i32::MAX)?; // COMPLETE
        let top_scores_manager = TopScoreDocCollectorManager::with_num_hits(10, 1)?; // TOP_SCORES

        let complete = searcher.search_with_manager(Arc::clone(&query), &complete_manager)?;
        let top_scores = searcher.search_with_manager(Arc::clone(&query), &top_scores_manager)?;
        CheckHits::check_equal(&*query, &complete.score_docs, &top_scores.score_docs);

        let filter_term = random().next_int_bound(15);
        let mut filtered = BooleanQueryBuilder::new();
        filtered.add(Arc::clone(&query), Occur::MUST)?;
        filtered.add(
            Arc::new(TermQuery::new(t("foo", &filter_term.to_string()))),
            Occur::FILTER,
        )?;
        let filtered_query: Arc<dyn Query> = Arc::new(filtered.build());

        let complete_manager = TopScoreDocCollectorManager::with_num_hits(10, i32::MAX)?; // COMPLETE
        let top_scores_manager = TopScoreDocCollectorManager::with_num_hits(10, 1)?; // TOP_SCORES

        let complete =
            searcher.search_with_manager(Arc::clone(&filtered_query), &complete_manager)?;
        let top_scores = searcher.search_with_manager(filtered_query, &top_scores_manager)?;
        CheckHits::check_equal(&*query, &complete.score_docs, &top_scores.score_docs);
    }
    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Port of `TestSynonymQuery.testRewrite`.
fn test_rewrite() -> Result<()> {
    let searcher =
        IndexSearcher::new(Arc::new(MultiReader::new(Vec::new(), true)?) as Arc<dyn IndexReader>)?;

    // zero length SynonymQuery is rewritten
    let q: SynonymQuery = builder("f").build();
    Assert::assert_true(q.get_terms().is_empty());
    assert_query_equals(
        &*searcher.rewrite(Arc::new(q))?,
        &MatchNoDocsQuery::instance(),
    );

    // non-boosted single term SynonymQuery is rewritten
    let q = builder("f")
        .add_term_with_boost(&Term::empty("f"), 1f32)?
        .build();
    assert_equals(&q.get_terms().len(), &1);
    assert_query_equals(
        &*searcher.rewrite(Arc::new(q))?,
        &TermQuery::new(Term::empty("f")),
    );

    // boosted single term SynonymQuery is not rewritten
    let q = builder("f")
        .add_term_with_boost(&Term::empty("f"), 0.8f32)?
        .build();
    assert_equals(&q.get_terms().len(), &1);
    assert_query_equals(&*searcher.rewrite(Arc::new(q.clone()))?, &q);

    // multiple term SynonymQuery is not rewritten
    let q = builder("f")
        .add_term_with_boost(&Term::empty("f"), 1f32)?
        .add_term_with_boost(&Term::empty("f"), 1f32)?
        .build();
    assert_equals(&q.get_terms().len(), &2);
    assert_query_equals(&*searcher.rewrite(Arc::new(q.clone()))?, &q);
    Ok(())
}
