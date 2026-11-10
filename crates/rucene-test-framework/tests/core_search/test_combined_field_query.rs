//! Port of `lucene/core/src/test/org/apache/lucene/search/TestCombinedFieldQuery.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::document::{Document, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_reader::MultiReader;
use rucene::index::postings_enum::DocAndFloatFeatureBuffer;
use rucene::index::{FieldInvertState, IndexReader};
use rucene::search::combined_field_query::Builder as CombinedFieldQueryBuilder;
use rucene::search::index_searcher::{IndexSearcher, IndexSearcherOverrides};
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::similarities::{
    BM25Similarity, BooleanSimilarity, ClassicSimilarity, CollectionStatistics,
    LMDirichletSimilarity, LMJelinekMercerSimilarity, SimScorer, Similarity, TermStatistics,
};
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::{
    query_to_string, Query, ScoreMode, TotalHits, TotalHitsRelation, NO_MORE_DOCS,
};
use rucene::store::mmap::MMapDirectory;
use rucene::store::Directory;
use rucene::util::{Bits, MatchNoBits};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::tq;
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test, IAE};

/// `TestCombinedFieldQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestCombinedFieldQuery",
        vec![
            test("testInvalid", test_invalid),
            test("testRewrite", test_rewrite),
            test("testEqualsAndHashCode", test_equals_and_hash_code),
            test("testToString", test_to_string),
            test("testSameScore", test_same_score),
            test(
                "testScoringWithMultipleFieldTermsMatch",
                test_scoring_with_multiple_field_terms_match,
            ),
            test("testNormsDisabled", test_norms_disabled),
            test("testCopyField", test_copy_field),
            test(
                "testCopyFieldWithSingleField",
                test_copy_field_with_single_field,
            ),
            test(
                "testCopyFieldWithMissingFields",
                test_copy_field_with_missing_fields,
            ),
            test("testDocWithNegativeNorms", test_doc_with_negative_norms),
            test(
                "testMultipleDocsNegativeNorms",
                test_multiple_docs_negative_norms,
            ),
            test(
                "testOverrideCollectionStatistics",
                test_override_collection_statistics,
            ),
            test("testNextDocsAndScores", test_next_docs_and_scores),
        ],
    )
}

/// `new CombinedFieldQuery.Builder(term).addField(f, w)...build()`, a
/// weight of `None` standing for `addField(f)`.
fn cfq(term: &str, fields: &[(&str, Option<f32>)]) -> Result<Arc<dyn Query>> {
    let mut b = CombinedFieldQueryBuilder::new(term);
    for (f, w) in fields {
        match w {
            Some(w) => b.add_field_with_weight(*f, *w)?,
            None => b.add_field(*f)?,
        };
    }
    Ok(Arc::new(b.build()?))
}

fn test_invalid() -> Result<()> {
    let mut builder = CombinedFieldQueryBuilder::new("foo");
    let exc = expect_throws(IAE, || {
        builder.add_field_with_weight("foo", 0.5).map(|_| ())
    });
    assert_equals(
        exc.message().unwrap_or_default(),
        "weight must be greater or equal to 1",
    );
    Ok(())
}

fn test_rewrite() -> Result<()> {
    let mut builder = CombinedFieldQueryBuilder::new("foo");
    let reader: Arc<dyn IndexReader> = Arc::new(MultiReader::new(Vec::new(), true)?);
    let searcher = IndexSearcher::new(reader)?;
    let actual = searcher.rewrite(Arc::new(builder.build()?))?;
    assert_query_equals(
        &(Arc::new(MatchNoDocsQuery::instance()) as Arc<dyn Query>),
        &actual,
    );
    builder.add_field_with_weight("field", 1.0)?;
    let query: Arc<dyn Query> = Arc::new(builder.build()?);
    let actual = searcher.rewrite(Arc::new(builder.build()?))?;
    assert_query_equals(&query, &actual);
    Ok(())
}

fn test_equals_and_hash_code() -> Result<()> {
    let query1 = cfq("value", &[("field1", None), ("field2", None)])?;

    let query2 = cfq("value", &[("field1", None), ("field2", Some(1.3))])?;
    Assert::assert_false(query1.query_eq(&*query2));
    Assert::assert_true(query1.query_hash() != query2.query_hash());

    let query3 = cfq("value", &[("field3", None), ("field4", None)])?;
    Assert::assert_false(query1.query_eq(&*query3));
    Assert::assert_true(query1.query_hash() != query2.query_hash());

    let duplicate_query1 = cfq("value", &[("field1", None), ("field2", None)])?;
    assert_query_equals(&query1, &duplicate_query1);
    assert_equals(&query1.query_hash(), &duplicate_query1.query_hash());
    Ok(())
}

fn test_to_string() -> Result<()> {
    let mut builder = CombinedFieldQueryBuilder::new("bar");
    assert_equals(
        "CombinedFieldQuery(()(bar))",
        query_to_string(&builder.build()?).as_str(),
    );
    builder.add_field_with_weight("foo", 1.0)?;
    assert_equals(
        "CombinedFieldQuery((foo)(bar))",
        query_to_string(&builder.build()?).as_str(),
    );
    builder.add_field_with_weight("title", 3.0)?;
    assert_equals(
        "CombinedFieldQuery((foo title^3.0)(bar))",
        query_to_string(&builder.build()?).as_str(),
    );
    Ok(())
}

/// `new IndexWriterConfig()` with `similarity`.
fn iwc_with(similarity: Arc<dyn Similarity>) -> Result<IndexWriterConfig> {
    let iwc = IndexWriterConfig::new();
    iwc.set_similarity(Some(similarity))?;
    Ok(iwc)
}

fn text(name: &str, value: &str) -> Result<Box<TextField>> {
    Ok(Box::new(TextField::new(
        name,
        value.to_string(),
        Store::NO,
    )?))
}

fn test_same_score() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let similarity = random_compatible_similarity()?;

    let iwc = iwc_with(Arc::clone(&similarity))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("f", "a".into(), Store::NO)?));
    w.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("g", "a".into(), Store::NO)?));
    for _i in 0..10 {
        w.add_document(&doc)?;
    }

    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(similarity);
    let query = cfq("a", &[("f", Some(1.0)), ("g", Some(1.0))])?;
    let collector_manager =
        TopScoreDocCollectorManager::with_num_hits(reader.num_docs().min(i32::MAX), i32::MAX)?;
    let top_docs = searcher.search_with_manager(query, &collector_manager)?;
    assert_equals(
        &TotalHits::new(11, TotalHitsRelation::EQUAL_TO)?,
        &top_docs.total_hits,
    );
    // All docs must have the same score
    for i in 0..top_docs.score_docs.len() {
        assert_equals_delta(
            top_docs.score_docs[0].score as f64,
            top_docs.score_docs[i].score as f64,
            0.0,
        );
    }

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// The documents of `testScoringWithMultipleFieldTermsMatch` and
/// `testNextDocsAndScores` (the same loop in both).
fn add_multiple_field_terms_docs(w: &RandomIndexWriter, num_match_doc: i32) -> Result<()> {
    // adding potentially matching doc
    for _i in 0..num_match_doc {
        let mut doc = Document::new();

        let mut freq_a = random().next_int_bound(20) + 1;
        for _j in 0..freq_a {
            doc.add(text("a", "foo")?);
        }

        freq_a = random().next_int_bound(20) + 1;
        if RandomizedTest::random_boolean() {
            for j in 0..freq_a {
                doc.add(text("a", &format!("foo{j}"))?);
            }
        }

        freq_a = random().next_int_bound(20) + 1;
        for _j in 0..freq_a {
            doc.add(text("a", "zoo")?);
        }

        let mut freq_b = random().next_int_bound(20) + 1;
        for _j in 0..freq_b {
            doc.add(text("b", "zoo")?);
        }

        freq_b = random().next_int_bound(20) + 1;
        if RandomizedTest::random_boolean() {
            for j in 0..freq_b {
                doc.add(text("b", &format!("zoo{j}"))?);
            }
        }

        let freq_c = random().next_int_bound(20) + 1;
        for j in 0..freq_c {
            doc.add(text("c", &format!("bla{j}"))?);
        }
        w.add_document(&doc)?;
    }
    Ok(())
}

fn test_scoring_with_multiple_field_terms_match() -> Result<()> {
    let num_match_doc = RandomizedTest::random_int_between(100, 500);
    let num_hits = RandomizedTest::random_int_between(1, 100);
    let boost1 = 1.max(random().next_int_bound(5));
    let boost2 = 1.max(random().next_int_bound(5));

    let mut dir = LuceneTestCase::new_directory();
    let similarity = random_compatible_similarity()?;

    let iwc = iwc_with(Arc::clone(&similarity))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    add_multiple_field_terms_docs(&w, num_match_doc)?;

    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(similarity);

    let query = cfq(
        "foo",
        &[("a", Some(boost1 as f32)), ("b", Some(boost2 as f32))],
    )?;

    let complete_manager = TopScoreDocCollectorManager::with_num_hits(num_hits, i32::MAX)?;

    searcher.search_with_manager(query, &complete_manager)?;

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_norms_disabled() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let similarity = random_compatible_similarity()?;

    let iwc = iwc_with(similarity)?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("a", "value".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("b", "value".into(), Store::NO)?));
    doc.add(text("c", "value")?);
    w.add_document(&doc)?;
    w.commit()?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("a", "value".into(), Store::NO)?));
    doc.add(text("c", "value")?);
    w.add_document(&doc)?;

    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let search_similarity = random_compatible_similarity()?;
    searcher.set_similarity(search_similarity);
    let collector_manager = TopScoreDocCollectorManager::with_num_hits(10, 10)?;

    let query = cfq("value", &[("a", Some(1.0)), ("b", Some(1.0))])?;
    let top_docs = searcher.search_with_manager(query, &collector_manager)?;
    assert_equals(
        &TotalHits::new(2, TotalHitsRelation::EQUAL_TO)?,
        &top_docs.total_hits,
    );

    let invalid_query = cfq("value", &[("b", Some(1.0)), ("c", Some(1.0))])?;
    let e = expect_throws(IAE, || {
        searcher.search_with_manager(Arc::clone(&invalid_query), &collector_manager)
    });
    Assert::assert_true(
        e.message()
            .unwrap_or_default()
            .contains("requires norms to be consistent across fields"),
    );

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_copy_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let similarity = random_compatible_similarity()?;

    let iwc = iwc_with(Arc::clone(&similarity))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let num_match = LuceneTestCase::at_least(10);
    let boost1 = 1.max(random().next_int_bound(5));
    let boost2 = 1.max(random().next_int_bound(5));
    for _i in 0..num_match {
        let mut doc = Document::new();
        if random().next_boolean() {
            doc.add(text("a", "baz")?);
            doc.add(text("b", "baz")?);
            for _k in 0..boost1 + boost2 {
                doc.add(text("ab", "baz")?);
            }
            w.add_document(&doc)?;
            doc.clear();
        }
        let freq_a = random().next_int_bound(5) + 1;
        for _j in 0..freq_a {
            doc.add(text("a", "foo")?);
        }
        let freq_b = random().next_int_bound(5) + 1;
        for _j in 0..freq_b {
            doc.add(text("b", "foo")?);
        }
        let freq_ab = freq_a * boost1 + freq_b * boost2;
        for _j in 0..freq_ab {
            doc.add(text("ab", "foo")?);
        }
        w.add_document(&doc)?;
    }
    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    searcher.set_similarity(similarity);
    let query = cfq(
        "foo",
        &[("a", Some(boost1 as f32)), ("b", Some(boost2 as f32))],
    )?;

    check_expected_hits(&searcher, num_match, query, tq("ab", "foo"))?;

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// `new MMapDirectory(createTempDir())`, shared as the `Directory` the
/// writer takes.
fn mmap_dir() -> Result<Arc<dyn Directory>> {
    Ok(Arc::new(MMapDirectory::open(
        LuceneTestCase::create_temp_dir(),
    )?))
}

/// `dir.close()` on a directory the test shares: closed once the test holds
/// the only handle.
fn close_shared_dir(mut dir: Arc<dyn Directory>) -> Result<()> {
    match Arc::get_mut(&mut dir) {
        Some(d) => d.close(),
        None => Ok(()),
    }
}

fn test_copy_field_with_single_field() -> Result<()> {
    let dir = mmap_dir()?;
    let similarity = random_compatible_similarity()?;

    let iwc = iwc_with(Arc::clone(&similarity))?;
    let w = RandomIndexWriter::new_with_config(&*random(), Arc::clone(&dir), iwc)?;

    let boost = 1.max(random().next_int_bound(5));
    let num_match = LuceneTestCase::at_least(10);
    for _i in 0..num_match {
        let mut doc = Document::new();
        let freq_a = random().next_int_bound(5) + 1;
        for _j in 0..freq_a {
            doc.add(text("a", "foo")?);
        }

        let freq_b = freq_a * boost;
        for _j in 0..freq_b {
            doc.add(text("b", "foo")?);
        }

        w.add_document(&doc)?;
    }

    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(similarity);
    let query = cfq("foo", &[("a", Some(boost as f32))])?;

    check_expected_hits(&searcher, num_match, query, tq("b", "foo"))?;

    reader.close()?;
    w.close()?;
    drop(searcher);
    drop(reader);
    drop(w);
    close_shared_dir(dir)?;
    Ok(())
}

fn test_copy_field_with_missing_fields() -> Result<()> {
    let dir = mmap_dir()?;
    let similarity = random_compatible_similarity()?;

    let iwc = iwc_with(Arc::clone(&similarity))?;
    let w = RandomIndexWriter::new_with_config(&*random(), Arc::clone(&dir), iwc)?;

    let boost1 = 1.max(random().next_int_bound(5));
    let boost2 = 1.max(random().next_int_bound(5));
    let num_match = LuceneTestCase::at_least(10);
    for _i in 0..num_match {
        let mut doc = Document::new();
        let freq_a = random().next_int_bound(5) + 1;
        for _j in 0..freq_a {
            doc.add(text("a", "foo")?);
        }

        // Choose frequencies such that sometimes we don't add field B
        let freq_b = random().next_int_bound(3);
        for _j in 0..freq_b {
            doc.add(text("b", "foo")?);
        }

        let freq_ab = freq_a * boost1 + freq_b * boost2;
        for _j in 0..freq_ab {
            doc.add(text("ab", "foo")?);
        }

        w.add_document(&doc)?;
    }

    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(similarity);
    let query = cfq(
        "foo",
        &[("a", Some(boost1 as f32)), ("b", Some(boost2 as f32))],
    )?;

    check_expected_hits(&searcher, num_match, query, tq("ab", "foo"))?;

    reader.close()?;
    w.close()?;
    drop(searcher);
    drop(reader);
    drop(w);
    close_shared_dir(dir)?;
    Ok(())
}

fn random_compatible_similarity() -> Result<Arc<dyn Similarity>> {
    let list: Vec<Arc<dyn Similarity>> = vec![
        Arc::new(BM25Similarity::new()),
        Arc::new(BooleanSimilarity::new()),
        Arc::new(ClassicSimilarity::new()),
        Arc::new(LMDirichletSimilarity::new()),
        Arc::new(LMJelinekMercerSimilarity::with_lambda(0.1)?),
    ];
    Ok(Arc::clone(RandomPicks::random_from_list(&*random(), &list)))
}

fn check_expected_hits(
    searcher: &IndexSearcher,
    num_hits: i32,
    first_query: Arc<dyn Query>,
    second_query: Arc<dyn Query>,
) -> Result<()> {
    let collector_manager = TopScoreDocCollectorManager::with_num_hits(num_hits, i32::MAX)?;

    let first_top_docs =
        searcher.search_with_manager(Arc::clone(&first_query), &collector_manager)?;
    assert_equals(&(num_hits as i64), &first_top_docs.total_hits.value());

    let collector_manager = TopScoreDocCollectorManager::with_num_hits(num_hits, i32::MAX)?;
    let second_top_docs = searcher.search_with_manager(second_query, &collector_manager)?;
    CheckHits::check_equal(
        &*first_query,
        &second_top_docs.score_docs,
        &first_top_docs.score_docs,
    );
    Ok(())
}

/// The documents of the negative-norms tests, indexed under
/// `NegativeNormSimilarity`, searched with `BM25Similarity` for
/// `foo` over `f` and `g`; `expected` are the doc IDs in order.
fn negative_norms(docs: &[(&str, &str)], expected: &[i32]) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = iwc_with(Arc::new(NegativeNormSimilarity::default()))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let query_string = "foo";

    for (f, g) in docs {
        let mut doc = Document::new();
        doc.add(text("f", f)?);
        doc.add(text("g", g)?);
        w.add_document(&doc)?;
    }

    let reader = w.get_reader()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(BM25Similarity::new()));
    let query = cfq(query_string, &[("f", None), ("g", None)])?;
    let top_docs = searcher.search(query, 10)?;
    CheckHits::check_doc_ids("queried docs do not match", expected, &top_docs.score_docs);

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_doc_with_negative_norms() -> Result<()> {
    // both fields must contain tokens that match the query string "foo"
    negative_norms(&[("foo", "foo baz")], &[0])
}

fn test_multiple_docs_negative_norms() -> Result<()> {
    // add another match on the query string to the second doc
    // Return doc1 ahead of doc0 since its tf is higher
    negative_norms(&[("foo", "foo baz"), ("foo is foo", "foo baz")], &[1, 0])
}

/// `NegativeNormSimilarity extends Similarity`.
///
/// **Divergence from Lucene 10.5.0.** Java's `scorer` returns
/// `new BM25Similarity().scorer(...)`; a Rucene `SimScorer` borrows the
/// similarity that made it, so the default `BM25Similarity` is held by the
/// instance instead of being created per call. It is stateless, so the
/// scorers are the same.
#[derive(Debug, Default)]
struct NegativeNormSimilarity {
    bm25: BM25Similarity,
}

impl Similarity for NegativeNormSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestCombinedFieldQuery$NegativeNormSimilarity"
    }

    fn compute_norm(&self, _state: &FieldInvertState) -> Result<i64> {
        Ok(-128)
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        self.bm25.scorer(boost, collection_stats, term_stats)
    }
}

/// The anonymous `IndexSearcher` of `testOverrideCollectionStatistics`.
#[derive(Debug)]
struct ExtraStatsSearcher {
    extra_max_doc: i32,
    extra_doc_count: i32,
    extra_sum_doc_freq: i32,
    extra_sum_total_term_freq_a: i32,
    extra_sum_total_term_freq_b: i32,
    extra_sum_total_term_freq_ab: i32,
}

impl IndexSearcherOverrides for ExtraStatsSearcher {
    fn collection_statistics(
        &self,
        searcher: &IndexSearcher,
        field: &str,
    ) -> Result<Option<CollectionStatistics>> {
        // Java dereferences the result without a null check.
        let shard_statistics = searcher
            .super_collection_statistics(field)?
            .expect("java.lang.NullPointerException: shardStatistics");
        let extra_sum_total_term_freq = if field == "a" {
            self.extra_sum_total_term_freq_a
        } else if field == "b" {
            self.extra_sum_total_term_freq_b
        } else if field == "ab" {
            self.extra_sum_total_term_freq_ab
        } else {
            Assert::fail("should never be called")
        };
        Ok(Some(CollectionStatistics::new(
            field,
            shard_statistics.max_doc() + self.extra_max_doc as i64,
            shard_statistics.doc_count() + self.extra_doc_count as i64,
            shard_statistics.sum_total_term_freq() + extra_sum_total_term_freq as i64,
            shard_statistics.sum_doc_freq() + self.extra_sum_doc_freq as i64,
        )?))
    }
}

fn test_override_collection_statistics() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let similarity = random_compatible_similarity()?;
    let iwc = iwc_with(Arc::clone(&similarity))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let num_match = LuceneTestCase::at_least(10);
    for _i in 0..num_match {
        let mut doc = Document::new();
        if random().next_boolean() {
            doc.add(text("a", "baz")?);
            doc.add(text("b", "baz")?);
            for _k in 0..2 {
                doc.add(text("ab", "baz")?);
            }
            w.add_document(&doc)?;
            doc.clear();
        }
        let freq_a = random().next_int_bound(5) + 1;
        for _j in 0..freq_a {
            doc.add(text("a", "foo")?);
        }
        let freq_b = random().next_int_bound(5) + 1;
        for _j in 0..freq_b {
            doc.add(text("b", "foo")?);
        }
        let freq_ab = freq_a + freq_b;
        for _j in 0..freq_ab {
            doc.add(text("ab", "foo")?);
        }
        w.add_document(&doc)?;
    }

    let reader = w.get_reader()?;

    let extra_max_doc = RandomizedTest::random_int_between(0, 10);
    let extra_doc_count = RandomizedTest::random_int_between(0, extra_max_doc);
    let extra_sum_doc_freq = extra_doc_count + RandomizedTest::random_int_between(0, 10);

    let extra_sum_total_term_freq_a =
        extra_sum_doc_freq + RandomizedTest::random_int_between(0, 10);
    let extra_sum_total_term_freq_b =
        extra_sum_doc_freq + RandomizedTest::random_int_between(0, 10);
    let extra_sum_total_term_freq_ab = extra_sum_total_term_freq_a + extra_sum_total_term_freq_b;

    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_overrides(Arc::new(ExtraStatsSearcher {
        extra_max_doc,
        extra_doc_count,
        extra_sum_doc_freq,
        extra_sum_total_term_freq_a,
        extra_sum_total_term_freq_b,
        extra_sum_total_term_freq_ab,
    }));
    searcher.set_similarity(similarity);
    let query = cfq("foo", &[("a", None), ("b", None)])?;

    check_expected_hits(&searcher, num_match, query, tq("ab", "foo"))?;

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_next_docs_and_scores() -> Result<()> {
    let num_match_doc = RandomizedTest::random_int_between(100, 500);
    let boost1 = 1.max(random().next_int_bound(5));
    let boost2 = 1.max(random().next_int_bound(5));

    let mut dir = LuceneTestCase::new_directory();
    let similarity = random_compatible_similarity()?;

    let iwc = iwc_with(Arc::clone(&similarity))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    add_multiple_field_terms_docs(&w, num_match_doc)?;

    w.force_merge(1)?;

    let reader = LuceneTestCase::get_only_leaf_reader(w.get_reader()? as Arc<dyn IndexReader>)?;
    let mut searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&reader), false);
    searcher.set_similarity(similarity);

    let query = cfq(
        "foo",
        &[("a", Some(boost1 as f32)), ("b", Some(boost2 as f32))],
    )?;

    let weight = searcher.create_weight(query, ScoreMode::TOP_SCORES, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    let context = &leaves[0];
    let live_docs: Option<Box<dyn Bits>> = context.leaf_reader().get_live_docs();

    let mut scorer1 = weight.scorer(context)?.expect("scorer");
    let mut scorer2 = weight.scorer(context)?.expect("scorer");
    scorer1.iterator().next_doc()?;
    scorer2.iterator().next_doc()?;
    let mut buffer = DocAndFloatFeatureBuffer::new();
    loop {
        let cur_doc = scorer2.iterator().doc_id();
        let up_to = TestUtil::next_int(
            &*random(),
            cur_doc,
            (i32::MAX as i64).min(cur_doc as i64 + 512) as i32,
        );
        scorer1.next_docs_and_scores(up_to, live_docs.as_deref(), &mut buffer)?;
        assert_equals(&(buffer.size == 0), &(cur_doc >= up_to));

        for i in 0..buffer.size {
            while live_docs
                .as_deref()
                .is_some_and(|l| !l.get(scorer2.iterator().doc_id() as usize))
            {
                scorer2.iterator().next_doc()?;
            }
            assert_equals(&scorer2.iterator().doc_id(), &buffer.docs[i]);
            assert_equals_delta(scorer2.score()? as f64, buffer.features[i] as f64, 0.0);
            scorer2.iterator().next_doc()?;
        }

        assert_equals(&scorer2.iterator().doc_id(), &scorer1.iterator().doc_id());
        if scorer1.iterator().doc_id() == NO_MORE_DOCS {
            break;
        }
    }

    let mut scorer3 = weight.scorer(context)?.expect("scorer");
    scorer3.iterator().next_doc()?;
    let match_no_bits = MatchNoBits::new(context.leaf_reader().max_doc() as usize);
    scorer3.next_docs_and_scores(NO_MORE_DOCS, Some(&match_no_bits), &mut buffer)?;
    assert_equals(&0, &buffer.size);

    Arc::clone(&reader).as_index_reader().close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}
