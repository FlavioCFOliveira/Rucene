//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTermInSetQuery.java`.
//!
//! Java's `HashSet<BytesRef>` of terms is a vector that keeps the first
//! occurrence of each term: the draws are the same, and the query sorts its
//! terms, so the set's iteration order does not reach an assertion.

use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::generators::RandomStrings;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, KeywordField, SortedSetDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::filter_reader::{FilterLeafReader, FilterTerms};
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, LeafReader, Term, Terms, TermsEnum};
use rucene::search::abstract_multi_term_query_constant_score_wrapper::BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::doc_values_rewrite;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::sort::{Sort, SortField};
use rucene::search::term_query::TermQuery;
use rucene::search::{
    query_to_string, BooleanQueryBuilder, BoostQuery, ConstantScoreQuery, Occur, Query,
    QueryCachingPolicy, ScoreMode, TermInSetQuery, UsageTrackingQueryCachingPolicy,
};
use rucene::util::automaton::{ByteRunAutomaton, ByteRunnable};
use rucene::util::bytes_ref_array::BytesRefIterator;
use rucene::util::{Accountable, BytesRef};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::ram_usage_tester::{ModelBuilder, RamUsageTester};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::assert_query_equals;
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_not_null, assert_null, class, random, test,
};

/// `TestTermInSetQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTermInSetQuery",
        vec![
            test("testAllDocsInFieldTerm", test_all_docs_in_field_term),
            test("testDuel", test_duel),
            test(
                "testReturnsNullScoreSupplier",
                test_returns_null_score_supplier,
            ),
            test(
                "testSkipperOptimizationGapAssumption",
                test_skipper_optimization_gap_assumption,
            ),
            test("testHashCodeAndEquals", test_hash_code_and_equals),
            test("testSimpleEquals", test_simple_equals),
            test("testToString", test_to_string),
            test("testDedup", test_dedup),
            test("testOrderDoesNotMatter", test_order_does_not_matter),
            test("testRamBytesUsed", test_ram_bytes_used),
            test("testPullOneTermsEnum", test_pull_one_terms_enum),
            test("testBinaryToString", test_binary_to_string),
            test(
                "testIsConsideredCostlyByQueryCache",
                test_is_considered_costly_by_query_cache,
            ),
            test("testVisitor", test_visitor),
            test("testTermsIterator", test_terms_iterator),
        ],
    )
}

/// `newBytesRef(s)`.
fn nbr(s: &str) -> BytesRef {
    LuceneTestCase::new_bytes_ref_str(s)
}

/// `new BytesRef(s)`.
fn br(s: &str) -> BytesRef {
    BytesRef::from_char_sequence(s)
}

/// `set.add(value)` on a `HashSet` kept as a vector of distinct values.
fn set_add(set: &mut Vec<BytesRef>, value: BytesRef) {
    if !set.contains(&value) {
        set.push(value);
    }
}

/// Port of `TestTermInSetQuery.testAllDocsInFieldTerm`.
fn test_all_docs_in_field_term() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let field = "f";

    let dense_term = br(&TestUtil::random_analysis_string(&*random(), 10, true));

    let mut random_terms: Vec<BytesRef> = Vec::new();
    while random_terms.len() < BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD {
        set_add(
            &mut random_terms,
            br(&TestUtil::random_analysis_string(&*random(), 10, true)),
        );
    }
    debug_assert!(random_terms.len() == BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD);
    let other_terms: Vec<BytesRef> = random_terms;

    // Every doc with a value for `field` will contain `denseTerm`:
    let num_docs = 10 * other_terms.len();
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new_with_bytes(
            field,
            dense_term.clone(),
            Store::NO,
        )?));
        let sparse_term = other_terms[i % other_terms.len()].clone();
        doc.add(Box::new(StringField::new_with_bytes(
            field,
            sparse_term,
            Store::NO,
        )?));
        iw.add_document(&doc)?;
    }

    // Make sure there are some docs in the index that don't contain a value for the field at all:
    for _i in 0..100 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "foo",
            "bar".to_string(),
            Store::NO,
        )?));
    }

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let mut query_terms: Vec<BytesRef> = other_terms.clone();
    query_terms.push(dense_term);

    let query = TermInSetQuery::new(field, query_terms)?;
    let top_docs = searcher.search(Arc::new(query), num_docs as i32)?;
    assert_equals(&(num_docs as i64), &top_docs.total_hits.value());

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// Port of `TestTermInSetQuery.testDuel`.
fn test_duel() -> Result<()> {
    let iters = LuceneTestCase::at_least(2);
    let field = "f";
    for _iter in 0..iters {
        let mut all_terms: Vec<BytesRef> = Vec::new();
        let num_terms =
            TestUtil::next_int(&*random(), 1, 1 << TestUtil::next_int(&*random(), 1, 10));
        for _i in 0..num_terms {
            let value = TestUtil::random_analysis_string(&*random(), 10, true);
            all_terms.push(nbr(&value));
        }
        let dir = LuceneTestCase::new_directory();
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let num_docs = LuceneTestCase::at_least(10_000);
        for _i in 0..num_docs {
            let mut doc = Document::new();
            let term = all_terms[random().next_int_bound(all_terms.len() as i32) as usize].clone();
            doc.add(Box::new(StringField::new_with_bytes(
                field,
                term.clone(),
                Store::NO,
            )?));
            // Also include a doc values field with a skip-list so we can test doc-value rewrite as
            // well:
            doc.add(Box::new(SortedSetDocValuesField::indexed_field(
                field, term,
            )));
            iw.add_document(&doc)?;
        }
        if num_terms > 1 && random().next_boolean() {
            iw.delete_documents_by_query(Arc::new(TermQuery::new(Term::new(
                field,
                all_terms[0].clone(),
            ))))?;
        }
        iw.commit()?;
        let reader = iw.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        iw.close()?;

        if reader.num_docs() == 0 {
            // may occasionally happen if all documents got the same term
            drop(searcher);
            let r1 = reader.close();
            let r2 = dir.close();
            r1.and(r2)?;
            continue;
        }

        for _i in 0..100 {
            let boost = random().next_float() * 10f32;
            let num_query_terms =
                TestUtil::next_int(&*random(), 1, 1 << TestUtil::next_int(&*random(), 1, 8));
            let mut query_terms: Vec<BytesRef> = Vec::new();
            for _j in 0..num_query_terms {
                query_terms.push(
                    all_terms[random().next_int_bound(all_terms.len() as i32) as usize].clone(),
                );
            }
            let mut bq = BooleanQueryBuilder::new();
            for t in &query_terms {
                bq.add(
                    Arc::new(TermQuery::new(Term::new(field, t.clone()))),
                    Occur::SHOULD,
                )?;
            }
            let q1: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(Arc::new(bq.build())));
            let q2: Arc<dyn Query> = Arc::new(TermInSetQuery::new(field, query_terms.clone())?);
            let q3: Arc<dyn Query> = Arc::new(TermInSetQuery::with_rewrite_method(
                doc_values_rewrite(),
                field,
                query_terms,
            )?);
            assert_same_matches(
                &searcher,
                Arc::new(BoostQuery::new(Arc::clone(&q1), boost)?),
                Arc::new(BoostQuery::new(q2, boost)?),
                true,
            )?;
            assert_same_matches(
                &searcher,
                Arc::new(BoostQuery::new(q1, boost)?),
                Arc::new(BoostQuery::new(q3, boost)?),
                false,
            )?;
        }

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestTermInSetQuery.testReturnsNullScoreSupplier`.
fn test_returns_null_score_supplier() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        {
            let writer = IndexWriter::new(directory.as_directory(), IndexWriterConfig::new())?;
            let result = (|| -> Result<()> {
                for ch in 'a'..='z' {
                    let mut doc = Document::new();
                    doc.add(Box::new(KeywordField::new(
                        "id",
                        ch.to_string(),
                        Store::YES,
                    )?));
                    doc.add(Box::new(KeywordField::new(
                        "content",
                        ch.to_string(),
                        Store::YES,
                    )?));
                    writer.add_document(&doc)?;
                }
                Ok(())
            })();
            result.and(writer.close())?;
        }
        let reader = directory_reader::open(directory.as_directory())?;
        let result = (|| -> Result<()> {
            let mut terms: Vec<BytesRef> = Vec::new();
            for ch in 'a'..='z' {
                terms.push(nbr(&ch.to_string()));
            }
            let query2: Arc<dyn Query> = Arc::new(TermInSetQuery::new("content", terms)?);
            let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;

            {
                // query1 doesn't match any documents
                let query1: Arc<dyn Query> =
                    Arc::new(TermInSetQuery::new("id", vec![nbr("aaa"), nbr("bbb")])?);
                let mut query_builder = BooleanQueryBuilder::new();
                query_builder.add(Arc::clone(&query1), Occur::FILTER)?;
                query_builder.add(Arc::clone(&query2), Occur::FILTER)?;
                let bool_query: Arc<dyn Query> = Arc::new(query_builder.build());

                let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
                let ctx = &leaves[0];

                let weight1 =
                    searcher.create_weight(searcher.rewrite(query1)?, ScoreMode::COMPLETE, 1.0)?;
                let scorer_supplier1 = weight1.scorer_supplier(ctx)?;
                // as query1 doesn't match any documents, its scorerSupplier must be null
                assert_null(&scorer_supplier1.map(|_| "ScorerSupplier"));
                let weight = searcher.create_weight(
                    searcher.rewrite(bool_query)?,
                    ScoreMode::COMPLETE,
                    1.0,
                )?;
                // scorerSupplier of a bool query where query1 is mandatory must be null
                let scorer_supplier = weight.scorer_supplier(ctx)?;
                assert_null(&scorer_supplier.map(|_| "ScorerSupplier"));
            }
            {
                // query1 matches some documents
                let query1: Arc<dyn Query> = Arc::new(TermInSetQuery::new(
                    "id",
                    vec![nbr("aaa"), nbr("bbb"), nbr("b")],
                )?);
                let mut query_builder = BooleanQueryBuilder::new();
                query_builder.add(Arc::clone(&query1), Occur::FILTER)?;
                query_builder.add(Arc::clone(&query2), Occur::FILTER)?;
                let bool_query: Arc<dyn Query> = Arc::new(query_builder.build());

                let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
                let ctx = &leaves[0];

                let weight1 =
                    searcher.create_weight(searcher.rewrite(query1)?, ScoreMode::COMPLETE, 1.0)?;
                let scorer_supplier1 = weight1.scorer_supplier(ctx)?;
                // as query1 matches some documents, its scorerSupplier must not be null
                assert_not_null(&scorer_supplier1);
                let weight = searcher.create_weight(
                    searcher.rewrite(bool_query)?,
                    ScoreMode::COMPLETE,
                    1.0,
                )?;
                // scorerSupplier of a bool query where query1 is mandatory must not be null
                let scorer_supplier = weight.scorer_supplier(ctx)?;
                assert_not_null(&scorer_supplier);
            }
            Ok(())
        })();
        result.and(reader.close())
    })();
    result.and(directory.close())
}

/// Make sure the doc values skipper isn't making the incorrect assumption
/// that the min/max terms from a TermInSetQuery don't form a continuous range.
///
/// Port of `TestTermInSetQuery.testSkipperOptimizationGapAssumption`.
fn test_skipper_optimization_gap_assumption() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    // Index the first 10,000 docs all with the term "b" to get some skip list blocks with the range
    // [b, b]:
    for _i in 0..10_000 {
        let mut doc = Document::new();
        let term = br("b");
        doc.add(Box::new(SortedSetDocValuesField::new(
            "field",
            term.clone(),
        )));
        doc.add(Box::new(SortedSetDocValuesField::indexed_field(
            "idx_field",
            term,
        )));
        iw.add_document(&doc)?;
    }

    // Index a couple more docs with terms "a" and "c":
    let mut doc = Document::new();
    let term = br("a");
    doc.add(Box::new(SortedSetDocValuesField::new(
        "field",
        term.clone(),
    )));
    doc.add(Box::new(SortedSetDocValuesField::indexed_field(
        "idx_field",
        term,
    )));
    iw.add_document(&doc)?;
    let mut doc = Document::new();
    let term = br("c");
    doc.add(Box::new(SortedSetDocValuesField::new(
        "field",
        term.clone(),
    )));
    doc.add(Box::new(SortedSetDocValuesField::indexed_field(
        "idx_field",
        term,
    )));
    iw.add_document(&doc)?;

    iw.commit()?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    // Our query is for (or "a" "c") which should use a skip-list optimization to exclude blocks of
    // documents that fall outside the range [a, c]. We want to test that they don't incorrectly do
    // the inverse and include all docs in a block that fall within [a, c] (which is why we have
    // blocks of only "b" docs up-front):
    let query_terms = vec![br("a"), br("c")];
    let q1: Arc<dyn Query> = Arc::new(TermInSetQuery::with_rewrite_method(
        doc_values_rewrite(),
        "field",
        query_terms.clone(),
    )?);
    let q2: Arc<dyn Query> = Arc::new(TermInSetQuery::with_rewrite_method(
        doc_values_rewrite(),
        "idx_field",
        query_terms,
    )?);
    assert_same_matches(&searcher, q1, q2, false)?;

    drop(searcher);
    reader.close()?;
    dir.close()
}

/// `private void assertSameMatches(IndexSearcher searcher, Query q1, Query q2, boolean scores)`.
fn assert_same_matches(
    searcher: &IndexSearcher,
    q1: Arc<dyn Query>,
    q2: Arc<dyn Query>,
    scores: bool,
) -> Result<()> {
    let max_doc = searcher.get_index_reader().max_doc();
    // `Sort.RELEVANCE` and `Sort.INDEXORDER`.
    let sort = || -> Result<Sort> {
        if scores {
            Ok(Sort::new())
        } else {
            Sort::new_fields(vec![SortField::FIELD_DOC])
        }
    };
    let td1 = searcher.search_with_sort(q1, max_doc, &sort()?)?;
    let td2 = searcher.search_with_sort(q2, max_doc, &sort()?)?;
    assert_equals(&td1.total_hits.value(), &td2.total_hits.value());
    for i in 0..td1.score_docs.len() {
        assert_equals(&td1.score_docs[i].doc(), &td2.score_docs[i].doc());
        if scores {
            assert_equals_delta(
                f64::from(td1.score_docs[i].score()),
                f64::from(td2.score_docs[i].score()),
                10e-7,
            );
        }
    }
    Ok(())
}

/// Port of `TestTermInSetQuery.testHashCodeAndEquals`.
fn test_hash_code_and_equals() -> Result<()> {
    let num = LuceneTestCase::at_least(100);
    let mut terms: Vec<BytesRef> = Vec::new();
    let mut unique_terms: Vec<BytesRef> = Vec::new();
    for _i in 0..num {
        let string = TestUtil::random_realistic_unicode_string(&*random());
        terms.push(nbr(&string));
        set_add(&mut unique_terms, nbr(&string));
        let left = TermInSetQuery::new("field", unique_terms.clone())?;
        shuffle(&mut terms, &*random());
        let right = TermInSetQuery::new("field", terms.clone())?;
        assert_query_equals(&right, &left);
        assert_equals(&right.query_hash(), &left.query_hash());
        if unique_terms.len() > 1 {
            let mut as_list: Vec<BytesRef> = unique_terms.clone();
            as_list.remove(0);
            let not_equal = TermInSetQuery::new("field", as_list)?;
            Assert::assert_false(left.query_eq(&not_equal));
            Assert::assert_false(right.query_eq(&not_equal));
        }
    }

    let tq1 = TermInSetQuery::new("thing", vec![nbr("apple")])?;
    let tq2 = TermInSetQuery::new("thing", vec![nbr("orange")])?;
    Assert::assert_false(tq1.query_hash() == tq2.query_hash());

    // different fields with the same term should have differing hashcodes
    let tq1 = TermInSetQuery::new("thing", vec![nbr("apple")])?;
    let tq2 = TermInSetQuery::new("thing2", vec![nbr("apple")])?;
    Assert::assert_false(tq1.query_hash() == tq2.query_hash());
    Ok(())
}

/// `String.hashCode()`.
fn java_string_hash_code(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// Port of `TestTermInSetQuery.testSimpleEquals`.
fn test_simple_equals() -> Result<()> {
    // Two terms with the same hash code
    assert_equals(
        &java_string_hash_code("AaAaBB"),
        &java_string_hash_code("BBBBBB"),
    );
    let left = TermInSetQuery::new("id", vec![nbr("AaAaAa"), nbr("AaAaBB")])?;
    let right = TermInSetQuery::new("id", vec![nbr("AaAaAa"), nbr("BBBBBB")])?;
    Assert::assert_false(left.query_eq(&right));
    Ok(())
}

/// Port of `TestTermInSetQuery.testToString`.
fn test_to_string() -> Result<()> {
    let terms_query = TermInSetQuery::new("field1", vec![nbr("a"), nbr("b"), nbr("c")])?;
    assert_equals("field1:(a b c)", &query_to_string(&terms_query));
    Ok(())
}

/// Port of `TestTermInSetQuery.testDedup`.
fn test_dedup() -> Result<()> {
    let query1 = TermInSetQuery::new("foo", vec![nbr("bar")])?;
    let query2 = TermInSetQuery::new("foo", vec![nbr("bar"), nbr("bar")])?;
    QueryUtils::check_equal(&query1, &query2);
    Ok(())
}

/// Port of `TestTermInSetQuery.testOrderDoesNotMatter`.
fn test_order_does_not_matter() -> Result<()> {
    // order of terms if different
    let query1 = TermInSetQuery::new("foo", vec![nbr("bar"), nbr("baz")])?;
    let query2 = TermInSetQuery::new("foo", vec![nbr("baz"), nbr("bar")])?;
    QueryUtils::check_equal(&query1, &query2);
    Ok(())
}

/// Port of `TestTermInSetQuery.testRamBytesUsed`.
fn test_ram_bytes_used() -> Result<()> {
    let mut terms: Vec<BytesRef> = Vec::new();
    let num_terms = 10000 + random().next_int_bound(1000);
    for _i in 0..num_terms {
        terms.push(nbr(&RandomStrings::random_unicode_of_length(
            &*random(),
            10,
        )));
    }
    let query = TermInSetQuery::new("f", terms)?;
    let model = ModelBuilder::new().term_in_set_query(&query);
    let actual_ram_bytes_used = RamUsageTester::ram_used(&*model);
    let expected_ram_bytes_used = query.ram_bytes_used();
    // error margin within 5%
    assert_equals_delta(
        expected_ram_bytes_used as f64,
        actual_ram_bytes_used as f64,
        actual_ram_bytes_used as f64 / 20.0,
    );
    Ok(())
}

// -----------------------------------------------------------------------------
// TermsCountingDirectoryReaderWrapper
// -----------------------------------------------------------------------------

/// `new TermsCountingDirectoryReaderWrapper(in, counter)`: `private static
/// class TermsCountingDirectoryReaderWrapper extends FilterDirectoryReader`.
fn terms_counting_directory_reader_wrapper(
    r#in: Arc<dyn DirectoryReader>,
    counter: Arc<AtomicI32>,
) -> Result<Arc<dyn DirectoryReader>> {
    Ok(Arc::new(FilterDirectoryReader::new(
        r#in,
        Arc::new(TermsCountingSubReaderWrapper {
            counter: Arc::clone(&counter),
        }),
        Arc::new(TermsCountingDirectoryReaderWrapperKind { counter }),
    )?))
}

/// `TermsCountingDirectoryReaderWrapper.doWrapDirectoryReader` and its
/// `getReaderCacheHelper()`, which returns `null`.
struct TermsCountingDirectoryReaderWrapperKind {
    counter: Arc<AtomicI32>,
}

impl DirectoryReaderWrapper for TermsCountingDirectoryReaderWrapperKind {
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        terms_counting_directory_reader_wrapper(inner, Arc::clone(&self.counter))
    }

    fn subclass_simple_name(&self) -> &'static str {
        "TermsCountingDirectoryReaderWrapper"
    }

    fn reader_cache_helper_is_null(&self) -> bool {
        true
    }
}

/// `private static class TermsCountingSubReaderWrapper extends SubReaderWrapper`.
struct TermsCountingSubReaderWrapper {
    counter: Arc<AtomicI32>,
}

impl SubReaderWrapper for TermsCountingSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(TermsCountingLeafReaderWrapper {
            inner: FilterLeafReader::new(reader)?,
            counter: Arc::clone(&self.counter),
        }))
    }
}

/// The anonymous `FilterTerms` of `TermsCountingLeafReaderWrapper.terms`,
/// whose `iterator()` counts.
struct CountingTerms {
    inner: FilterTerms,
    counter: Arc<AtomicI32>,
}

impl Terms for CountingTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        self.inner.iterator()
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

/// `private static class TermsCountingLeafReaderWrapper extends FilterLeafReader`.
struct TermsCountingLeafReaderWrapper {
    inner: FilterLeafReader,
    counter: Arc<AtomicI32>,
}

impl fmt::Debug for TermsCountingLeafReaderWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TermsCountingLeafReaderWrapper")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TermsCountingLeafReaderWrapper {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for TermsCountingLeafReaderWrapper {
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
        None
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let Some(terms) = self.inner.terms(field)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(CountingTerms {
            inner: FilterTerms::new(terms),
            counter: Arc::clone(&self.counter),
        })))
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

/// Port of `TestTermInSetQuery.testPullOneTermsEnum`.
fn test_pull_one_terms_enum() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "foo",
        "1".to_string(),
        Store::NO,
    )?));
    w.add_document(&doc)?;
    let reader = w.get_reader()?;
    w.close()?;
    let counter = Arc::new(AtomicI32::new(0));
    let wrapped = terms_counting_directory_reader_wrapper(reader, Arc::clone(&counter))?;

    let mut terms: Vec<BytesRef> = Vec::new();
    // enough terms to avoid the rewrite
    let num_terms = TestUtil::next_int(
        &*random(),
        BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD as i32 + 1,
        100,
    );
    for _i in 0..num_terms {
        let term = nbr(&RandomStrings::random_unicode_of_codepoint_length(
            &*random(),
            10,
        ));
        terms.push(term);
    }

    assert_equals(
        &0,
        &IndexSearcher::new(Arc::clone(&wrapped) as Arc<dyn IndexReader>)?
            .count(Arc::new(TermInSetQuery::new("bar", terms.clone())?))?,
    );
    assert_equals(&0, &counter.load(Ordering::SeqCst)); // missing field
    IndexSearcher::new(Arc::clone(&wrapped) as Arc<dyn IndexReader>)?
        .count(Arc::new(TermInSetQuery::new("foo", terms)?))?;
    assert_equals(&1, &counter.load(Ordering::SeqCst));
    wrapped.close()?;
    dir.close()
}

/// Port of `TestTermInSetQuery.testBinaryToString`.
fn test_binary_to_string() -> Result<()> {
    let query = TermInSetQuery::new(
        "field",
        vec![LuceneTestCase::new_bytes_ref_bytes(&[0xff, 0xfe])],
    )?;
    assert_equals("field:([ff fe])", &query_to_string(&query));
    Ok(())
}

/// Port of `TestTermInSetQuery.testIsConsideredCostlyByQueryCache`.
fn test_is_considered_costly_by_query_cache() -> Result<()> {
    let query = TermInSetQuery::new("foo", vec![nbr("bar"), nbr("baz")])?;
    let policy = UsageTrackingQueryCachingPolicy::new()?;
    Assert::assert_false(policy.should_cache(&query)?);
    policy.on_use(&query);
    policy.on_use(&query);
    // cached after two uses
    Assert::assert_true(policy.should_cache(&query)?);
    Ok(())
}

/// The anonymous `QueryVisitor` of the singleton query.
struct SingletonVisitor;

impl QueryVisitor for SingletonVisitor {
    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        assert_equals(&1, &terms.len());
        assert_equals(&Term::new("field", nbr("term1")), &terms[0]);
    }

    fn consume_terms_matching(
        &mut self,
        _query: &dyn Query,
        _field: &str,
        _automaton: &dyn Fn() -> ByteRunAutomaton,
    ) {
        Assert::fail("Singleton TermInSetQuery should not try to build ByteRunAutomaton");
    }
}

/// The anonymous `QueryVisitor` of the multi-valued query.
struct AutomatonVisitor {
    terms: Vec<BytesRef>,
}

impl QueryVisitor for AutomatonVisitor {
    fn consume_terms(&mut self, _query: &dyn Query, _terms: &[Term]) {
        Assert::fail("TermInSetQuery with multiple terms should build automaton");
    }

    fn consume_terms_matching(
        &mut self,
        _query: &dyn Query,
        _field: &str,
        automaton: &dyn Fn() -> ByteRunAutomaton,
    ) {
        let a = automaton();
        let test = nbr("nonmatching");
        Assert::assert_false(a.run_range(&test.bytes, test.offset, test.length));
        for term in &self.terms {
            Assert::assert_true(a.run_range(&term.bytes, term.offset, term.length));
        }
    }
}

/// Port of `TestTermInSetQuery.testVisitor`.
fn test_visitor() -> Result<()> {
    // singleton reports back to consumeTerms()
    let singleton = TermInSetQuery::new("field", vec![nbr("term1")])?;
    singleton.visit(&mut SingletonVisitor);

    // multiple values built into automaton
    let mut terms: Vec<BytesRef> = Vec::new();
    for i in 0..100 {
        terms.push(nbr(&format!("term{i}")));
    }
    let t = TermInSetQuery::new("field", terms.clone())?;
    t.visit(&mut AutomatonVisitor { terms });
    Ok(())
}

/// Port of `TestTermInSetQuery.testTermsIterator`.
fn test_terms_iterator() -> Result<()> {
    let empty = TermInSetQuery::new("field", Vec::new())?;
    let mut it = empty.get_bytes_ref_iterator();
    assert_null(&it.next()?);

    let query = TermInSetQuery::new("field", vec![nbr("term1"), nbr("term2"), nbr("term3")])?;
    let mut it = query.get_bytes_ref_iterator();
    assert_equals(&Some(nbr("term1")), &it.next()?);
    assert_equals(&Some(nbr("term2")), &it.next()?);
    assert_equals(&Some(nbr("term3")), &it.next()?);
    assert_null(&it.next()?);
    Ok(())
}
