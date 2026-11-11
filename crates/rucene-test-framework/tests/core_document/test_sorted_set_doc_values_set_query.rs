//! Port of `lucene/core/src/test/org/apache/lucene/document/TestSortedSetDocValuesSetQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, SortedDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::{
    BooleanQueryBuilder, BoostQuery, ConstantScoreQuery, IndexSearcher, Query, TermQuery,
};
use rucene::util::BytesRef;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, assert_not_equals, class, random, test,
};

/// `TestSortedSetDocValuesSetQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestSortedSetDocValuesSetQuery",
        vec![
            test("testMissingTerms", test_missing_terms),
            test("testEquals", test_equals),
            test("testDuelTermsQuery", test_duel_terms_query),
            test("testApproximation", test_approximation),
        ],
    )
}

fn br(s: &str) -> BytesRef {
    BytesRef::from_char_sequence(s)
}

/// Port of `testMissingTerms`.
fn test_missing_terms() -> Result<()> {
    let field_name = "field1";
    let rd = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), rd.as_directory())?;
    for i in 0..100 {
        let mut doc = Document::new();
        let term = i * 10; // terms are units of 10;
        doc.add(Box::new(LuceneTestCase::new_string_field(
            field_name,
            &term.to_string(),
            Store::YES,
        )));
        doc.add(Box::new(SortedDocValuesField::new(
            field_name,
            br(&term.to_string()),
        )));
        w.add_document(&doc)?;
    }
    let reader = w.get_reader()?;
    w.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let num_docs = reader.num_docs();

    let terms = vec![br("5")];
    let results = searcher
        .search(
            SortedDocValuesField::new_slow_set_query(field_name, terms)?,
            num_docs,
        )?
        .score_docs;
    assert_equals_msg(Some("Must match nothing"), &0, &results.len());

    let terms = vec![br("10")];
    let results = searcher
        .search(
            SortedDocValuesField::new_slow_set_query(field_name, terms)?,
            num_docs,
        )?
        .score_docs;
    assert_equals_msg(Some("Must match 1"), &1, &results.len());

    let terms = vec![br("10"), br("20")];
    let results = searcher
        .search(
            SortedDocValuesField::new_slow_set_query(field_name, terms)?,
            num_docs,
        )?
        .score_docs;
    assert_equals_msg(Some("Must match 2"), &2, &results.len());

    (reader as Arc<dyn IndexReader>).close()?;
    rd.close()
}

/// `assertEquals(q1, q2)` / `assertNotEquals(q1, q2)` over queries.
fn query_equal(q1: &Arc<dyn Query>, q2: &Arc<dyn Query>) -> bool {
    q1.query_eq(q2.as_ref())
}

/// Port of `testEquals`.
fn test_equals() -> Result<()> {
    let bar = vec![br("bar")];
    let barbar = vec![br("bar"), br("bar")];
    let barbaz = vec![br("bar"), br("baz")];
    let bazbar = vec![br("baz"), br("bar")];
    let baz = vec![br("baz")];

    let q = |f: &str, v: &Vec<BytesRef>| SortedDocValuesField::new_slow_set_query(f, v.clone());
    assert_equals(&true, &query_equal(&q("foo", &bar)?, &q("foo", &bar)?));
    assert_equals(&true, &query_equal(&q("foo", &bar)?, &q("foo", &barbar)?));
    assert_equals(
        &true,
        &query_equal(&q("foo", &barbaz)?, &q("foo", &bazbar)?),
    );
    assert_not_equals(&true, &query_equal(&q("foo", &bar)?, &q("foo2", &bar)?));
    assert_not_equals(&true, &query_equal(&q("foo", &bar)?, &q("foo", &baz)?));
    Ok(())
}

/// The shared body of `testDuelTermsQuery` and `testApproximation`; with
/// `filtered`, each query is wrapped with a `FILTER` on `allTerms.get(0)`.
fn duel(filtered: bool) -> Result<()> {
    let iters = LuceneTestCase::at_least(2);
    for _iter in 0..iters {
        let mut all_terms: Vec<Term> = Vec::new();
        let bound = TestUtil::next_int(&*random(), 1, 10);
        let num_terms = TestUtil::next_int(&*random(), 1, 1 << bound);
        for _i in 0..num_terms {
            let value = TestUtil::random_analysis_string(&*random(), 10, true);
            all_terms.push(Term::from_text("f", &value));
        }
        let dir = LuceneTestCase::new_directory();
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let num_docs = LuceneTestCase::at_least(100);
        for _i in 0..num_docs {
            let mut doc = Document::new();
            let term = &all_terms[random().next_int_bound(all_terms.len() as i32) as usize];
            doc.add(Box::new(StringField::new(
                term.field(),
                term.text(),
                Store::NO,
            )?));
            doc.add(Box::new(SortedDocValuesField::new(
                term.field(),
                br(&term.text()),
            )));
            iw.add_document(&doc)?;
        }
        if num_terms > 1 && random().next_boolean() {
            iw.delete_documents_by_query(Arc::new(TermQuery::new(all_terms[0].clone())))?;
        }
        iw.commit()?;
        let reader = iw.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        iw.close()?;

        if reader.num_docs() == 0 {
            // may occasionally happen if all documents got the same term
            (reader as Arc<dyn IndexReader>).close()?;
            dir.close()?;
            continue;
        }

        for _i in 0..100 {
            let boost = random().next_float() * 10.0;
            let bound = TestUtil::next_int(&*random(), 1, 8);
            let num_query_terms = TestUtil::next_int(&*random(), 1, 1 << bound);
            let mut query_terms: Vec<Term> = Vec::new();
            for _j in 0..num_query_terms {
                query_terms.push(
                    all_terms[random().next_int_bound(all_terms.len() as i32) as usize].clone(),
                );
            }
            let mut bq = BooleanQueryBuilder::new();
            for term in &query_terms {
                bq.add(Arc::new(TermQuery::new(term.clone())), Occur::SHOULD)?;
            }
            let q1: Arc<dyn Query> = Arc::new(BoostQuery::new(
                Arc::new(ConstantScoreQuery::new(Arc::new(bq.build()))),
                boost,
            )?);
            let bytes_terms: Vec<BytesRef> =
                query_terms.iter().map(|t| t.bytes().clone()).collect();
            let q2: Arc<dyn Query> = Arc::new(BoostQuery::new(
                SortedDocValuesField::new_slow_set_query("f", bytes_terms)?,
                boost,
            )?);
            if filtered {
                let mut bq1 = BooleanQueryBuilder::new();
                bq1.add(q1, Occur::MUST)?;
                bq1.add(
                    Arc::new(TermQuery::new(all_terms[0].clone())),
                    Occur::FILTER,
                )?;
                let mut bq2 = BooleanQueryBuilder::new();
                bq2.add(q2, Occur::MUST)?;
                bq2.add(
                    Arc::new(TermQuery::new(all_terms[0].clone())),
                    Occur::FILTER,
                )?;
                assert_same_matches(
                    &searcher,
                    Arc::new(bq1.build()),
                    Arc::new(bq2.build()),
                    true,
                )?;
            } else {
                assert_same_matches(&searcher, q1, q2, true)?;
            }
        }

        (reader as Arc<dyn IndexReader>).close()?;
        dir.close()?;
    }
    Ok(())
}

/// Port of `testDuelTermsQuery`.
fn test_duel_terms_query() -> Result<()> {
    duel(false)
}

/// Port of `testApproximation`.
fn test_approximation() -> Result<()> {
    duel(true)
}

/// `assertSameMatches(searcher, q1, q2, scores)`.
fn assert_same_matches(
    searcher: &IndexSearcher,
    q1: Arc<dyn Query>,
    q2: Arc<dyn Query>,
    scores: bool,
) -> Result<()> {
    let max_doc = searcher.get_index_reader().max_doc();
    // `scores ? Sort.RELEVANCE : Sort.INDEXORDER`.
    let sort = if scores {
        Sort::new()
    } else {
        Sort::new_fields(vec![SortField::new(None, SortFieldType::Doc)?])?
    };
    let td1 = searcher.search_with_sort(q1, max_doc, &sort)?;
    let td2 = searcher.search_with_sort(q2, max_doc, &sort)?;
    assert_equals(&td1.total_hits.value(), &td2.total_hits.value());
    for i in 0..td1.score_docs.len() {
        assert_equals(
            &td1.score_docs[i].score_doc.doc,
            &td2.score_docs[i].score_doc.doc,
        );
        if scores {
            assert_equals_delta(
                td1.score_docs[i].score_doc.score as f64,
                td2.score_docs[i].score_doc.score as f64,
                10e-7,
            );
        }
    }
    Ok(())
}
