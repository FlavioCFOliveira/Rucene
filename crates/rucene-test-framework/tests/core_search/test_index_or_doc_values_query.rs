//! Port of `lucene/core/src/test/org/apache/lucene/search/TestIndexOrDocValuesQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{
    Document, LongField, LongPoint, NumericDocValuesField, SortedNumericDocValuesField, Store,
    StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::IndexReader;
use rucene::search::index_or_doc_values_query::IndexOrDocValuesQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQueryBuilder, MatchAllDocsQuery, MatchNoDocsQuery, Occur, Query, ScoreMode,
};
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::term;
use crate::core_support::{assert_equals, assert_not_null, assert_null, class, random, test};

/// `TestIndexOrDocValuesQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestIndexOrDocValuesQuery",
        vec![
            test(
                "testUseIndexForSelectiveQueries",
                test_use_index_for_selective_queries,
            ),
            test(
                "testUseIndexForSelectiveMultiValueQueries",
                test_use_index_for_selective_multi_value_queries,
            ),
            test("testQueryMatchesCount", test_query_matches_count),
            test("testQueryMatchesAllOrNone", test_query_matches_all_or_none),
        ],
    )
}

/// `new IndexWriter(dir, newIndexWriterConfig().setCodec(TestUtil.getDefaultCodec()))`:
/// relies on costs and `PointValues.estimateCost`, so it needs the default codec.
fn default_codec_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<IndexWriter>> {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_codec(TestUtil::get_default_codec())?;
    IndexWriter::new(dir.as_directory(), iwc)
}

/// `new BooleanQuery.Builder().add(new TermQuery(new Term("f1", f1)), MUST).add(q, MUST).build()`.
fn term_and(f1: &str, q: Arc<dyn Query>) -> Result<Arc<dyn Query>> {
    let mut b = BooleanQueryBuilder::new();
    b.add(Arc::new(TermQuery::new(term("f1", f1))), Occur::MUST)?;
    b.add(q, Occur::MUST)?;
    Ok(Arc::new(b.build()))
}

/// `searcher.createWeight(searcher.rewrite(q), COMPLETE, 1).scorer(leaves().get(0)).twoPhaseIterator() != null`.
fn uses_two_phase(searcher: &IndexSearcher, q: Arc<dyn Query>) -> Result<Option<()>> {
    let w = searcher.create_weight(searcher.rewrite(q)?, ScoreMode::COMPLETE, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    let mut s = w.scorer(&leaves[0])?.expect("scorer");
    Ok(s.two_phase_iterator().map(|_| ()))
}

/// Port of `TestIndexOrDocValuesQuery.testUseIndexForSelectiveQueries`.
fn test_use_index_for_selective_queries() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = default_codec_writer(&dir)?;
    for i in 0..2000 {
        let mut doc = Document::new();
        if i == 42 {
            doc.add(Box::new(StringField::new("f1", "bar".into(), Store::NO)?));
            doc.add(Box::new(LongPoint::new("f2", &[42])?));
            doc.add(Box::new(NumericDocValuesField::new("f2", 42)));
        } else if i == 100 {
            doc.add(Box::new(StringField::new("f1", "foo".into(), Store::NO)?));
            doc.add(Box::new(LongPoint::new("f2", &[2])?));
            doc.add(Box::new(NumericDocValuesField::new("f2", 2)));
        } else {
            doc.add(Box::new(StringField::new("f1", "bar".into(), Store::NO)?));
            doc.add(Box::new(LongPoint::new("f2", &[2])?));
            doc.add(Box::new(NumericDocValuesField::new("f2", 2)));
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_cache(None);

    // The term query is more selective, so the IndexOrDocValuesQuery should use doc values
    let q1 = term_and(
        "foo",
        Arc::new(IndexOrDocValuesQuery::new(
            LongPoint::new_exact_query("f2", 2)?,
            NumericDocValuesField::new_slow_range_query("f2", 2, 2),
        )),
    )?;
    QueryUtils::check_searcher(&*random(), &q1, Some(&searcher));

    assert_not_null(&uses_two_phase(&searcher, q1)?); // means we use doc values

    // The term query is less selective, so the IndexOrDocValuesQuery should use points
    let q2 = term_and(
        "bar",
        Arc::new(IndexOrDocValuesQuery::new(
            LongPoint::new_exact_query("f2", 42)?,
            NumericDocValuesField::new_slow_range_query("f2", 42, 42),
        )),
    )?;
    QueryUtils::check_searcher(&*random(), &q2, Some(&searcher));

    assert_null(&uses_two_phase(&searcher, q2)?); // means we use points

    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestIndexOrDocValuesQuery.testUseIndexForSelectiveMultiValueQueries`.
fn test_use_index_for_selective_multi_value_queries() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = default_codec_writer(&dir)?;
    let num_docs = LuceneTestCase::at_least(1000);
    for i in 0..num_docs {
        let mut doc = Document::new();
        if i < num_docs / 2 {
            doc.add(Box::new(StringField::new("f1", "bar".into(), Store::NO)?));
            for _j in 0..500 {
                doc.add(Box::new(LongField::new("f2", 42, Store::NO)));
            }
        } else if i == num_docs / 2 {
            doc.add(Box::new(StringField::new("f1", "foo".into(), Store::NO)?));
            doc.add(Box::new(LongField::new("f2", 2, Store::NO)));
        } else {
            doc.add(Box::new(StringField::new("f1", "bar".into(), Store::NO)?));
            for _j in 0..100 {
                doc.add(Box::new(LongField::new("f2", 2, Store::NO)));
            }
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_query_cache(None);

    // The term query is less selective, so the IndexOrDocValuesQuery should use points
    let q1 = term_and(
        "bar",
        Arc::new(IndexOrDocValuesQuery::new(
            LongPoint::new_exact_query("f2", 2)?,
            SortedNumericDocValuesField::new_slow_range_query("f2", 2, 2),
        )),
    )?;
    QueryUtils::check_searcher(&*random(), &q1, Some(&searcher));

    assert_null(&uses_two_phase(&searcher, q1)?); // means we use points

    // The term query is less selective, so the IndexOrDocValuesQuery should use points
    let q2 = term_and(
        "bar",
        Arc::new(IndexOrDocValuesQuery::new(
            LongPoint::new_exact_query("f2", 42)?,
            SortedNumericDocValuesField::new_slow_range_query("f2", 42, 42),
        )),
    )?;
    QueryUtils::check_searcher(&*random(), &q2, Some(&searcher));

    assert_null(&uses_two_phase(&searcher, q2)?); // means we use points

    // The term query is more selective, so the IndexOrDocValuesQuery should use doc values
    let q3 = term_and(
        "foo",
        Arc::new(IndexOrDocValuesQuery::new(
            LongPoint::new_exact_query("f2", 42)?,
            SortedNumericDocValuesField::new_slow_range_query("f2", 42, 42),
        )),
    )?;
    QueryUtils::check_searcher(&*random(), &q3, Some(&searcher));

    assert_not_null(&uses_two_phase(&searcher, q3)?); // means we use doc values

    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestIndexOrDocValuesQuery.testQueryMatchesCount`.
fn test_query_matches_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = default_codec_writer(&dir)?;
    let num_docs = random().next_int_bound(5000);
    for _i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LongPoint::new("f2", &[42])?));
        doc.add(Box::new(SortedNumericDocValuesField::new("f2", 42)));
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query: Arc<dyn Query> = Arc::new(IndexOrDocValuesQuery::new(
        LongPoint::new_exact_query("f2", 42)?,
        SortedNumericDocValuesField::new_slow_range_query("f2", 42, 42),
    ));
    QueryUtils::check_searcher(&*random(), &query, Some(&searcher));

    let search_count = searcher.count(Arc::clone(&query))?;
    let weight = searcher.create_weight(query, ScoreMode::COMPLETE, 1.0)?;
    let mut weight_count = 0;
    for leaf_reader_context in &(Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()? {
        weight_count += weight.count(leaf_reader_context)?;
    }
    assert_equals(&search_count, &weight_count);

    drop((weight, searcher));
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// `query.rewrite(searcher).getClass()` is `T`.
fn rewrites_to<T: 'static>(query: &Arc<dyn Query>, searcher: &IndexSearcher) -> Result<bool> {
    let rewritten = query
        .rewrite(searcher)?
        .unwrap_or_else(|| Arc::clone(query));
    Ok(rewritten.as_any().is::<T>())
}

/// Port of `TestIndexOrDocValuesQuery.testQueryMatchesAllOrNone`.
fn test_query_matches_all_or_none() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = default_codec_writer(&dir)?;
    let result = (|| -> Result<()> {
        let num_docs = TestUtil::next_int(&*random(), 1, 100);
        for i in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(LongPoint::new("f2", &[i64::from(i)])?));
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "f2",
                i64::from(i),
            )));
            w.add_document(&doc)?;
        }
        w.force_merge(1)?;

        let reader = directory_reader::open_with_writer(&*w)?;
        let inner = (|| -> Result<()> {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            let n = i64::from(num_docs);

            // range with all docs
            let query: Arc<dyn Query> = Arc::new(IndexOrDocValuesQuery::new(
                LongPoint::new_range_query("f2", &[0], &[n])?,
                SortedNumericDocValuesField::new_slow_range_query("f2", 0, n),
            ));
            QueryUtils::check_searcher(&*random(), &query, Some(&searcher));
            assert_equals(&true, &rewrites_to::<MatchAllDocsQuery>(&query, &searcher)?);

            // range with no docs
            let query: Arc<dyn Query> = Arc::new(IndexOrDocValuesQuery::new(
                LongPoint::new_range_query("f2", &[n + 1], &[n + 200])?,
                SortedNumericDocValuesField::new_slow_range_query("f2", n + 1, n + 200),
            ));
            QueryUtils::check_searcher(&*random(), &query, Some(&searcher));
            assert_equals(&true, &rewrites_to::<MatchNoDocsQuery>(&query, &searcher)?);
            Ok(())
        })();
        reader.close()?;
        inner
    })();
    w.close()?;
    dir.close()?;
    result
}
