//! Port of `lucene/core/src/test/org/apache/lucene/document/TestLongDistanceFeatureQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, LongField, Store};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_reader::MultiReader;
use rucene::index::IndexReader;
use rucene::search::{Query, ScoreDoc, TopScoreDocCollectorManager};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::search::{CheckHits, QueryUtils};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestLongDistanceFeatureQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestLongDistanceFeatureQuery",
        vec![
            test("testEqualsAndHashcode", test_equals_and_hashcode),
            test("testBasics", test_basics),
            test("testOverUnderFlow", test_over_under_flow),
            test("testMissingField", test_missing_field),
            test("testMissingValue", test_missing_value),
            test("testMultiValued", test_multi_valued),
            test("testRandom", test_random),
        ],
    )
}

fn q(field: &str, weight: f32, origin: i64, pivot: i64) -> Result<Arc<dyn Query>> {
    LongField::new_distance_feature_query(field, weight, origin, pivot)
}

/// Port of `testEqualsAndHashcode`.
fn test_equals_and_hashcode() -> Result<()> {
    let q1 = q("foo", 3.0, 10, 5)?;
    let q2 = q("foo", 3.0, 10, 5)?;
    QueryUtils::check_equal(q1.as_ref(), q2.as_ref());

    let q3 = q("bar", 3.0, 10, 5)?;
    QueryUtils::check_unequal(q1.as_ref(), q3.as_ref());

    let q4 = q("foo", 4.0, 10, 5)?;
    QueryUtils::check_unequal(q1.as_ref(), q4.as_ref());

    let q5 = q("foo", 3.0, 9, 5)?;
    QueryUtils::check_unequal(q1.as_ref(), q5.as_ref());

    let q6 = q("foo", 3.0, 10, 6)?;
    QueryUtils::check_unequal(q1.as_ref(), q6.as_ref());
    Ok(())
}

/// `new RandomIndexWriter(random(), dir,
/// newIndexWriterConfig().setMergePolicy(newLogMergePolicy(random().nextBoolean())))`.
fn writer(dir: &TestDirectory) -> Result<RandomIndexWriter> {
    let config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    )))?;
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)
}

/// `Document doc` holding `LongField field = new LongField("foo", 0L, Store.NO)`.
fn field_doc() -> Document {
    let mut doc = Document::new();
    let field = LongField::new("foo", 0, Store::NO);
    doc.add(Box::new(field));
    doc
}

/// `field.setLongValue(v)` on the `LongField` `doc` holds.
fn set_long_value(doc: &mut Document, v: i64) {
    doc.get_field_mut_as::<LongField>("foo")
        .expect("field")
        .set_value(v);
}

/// A document of `LongField("foo", v, Store.NO)` for each of `values`.
fn doc(values: &[i64]) -> Document {
    let mut doc = Document::new();
    for &v in values {
        doc.add(Box::new(LongField::new("foo", v, Store::NO)));
    }
    doc
}

/// `searcher.search(q, new TopScoreDocCollectorManager(n, 1)).scoreDocs`.
fn top(
    searcher: &rucene::search::IndexSearcher,
    q: &Arc<dyn Query>,
    n: i32,
) -> Result<Vec<ScoreDoc>> {
    let collector_manager = TopScoreDocCollectorManager::new(n, None, 1)?;
    Ok(searcher
        .search_with_manager(Arc::clone(q), &collector_manager)?
        .score_docs)
}

/// Port of `testBasics`.
fn test_basics() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut doc = field_doc();

    for v in [3, 12, 8, -1, 7] {
        set_long_value(&mut doc, v);
        w.add_document(&doc)?;
    }

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = q("foo", 3.0, 10, 5)?;
    let top_hits = top(&searcher, &query, 2)?;
    assert_equals(&2, &top_hits.len());
    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(1, (3f32 as f64 * (5. / (5. + 2.))) as f32),
            ScoreDoc::new(2, (3f32 as f64 * (5. / (5. + 2.))) as f32),
        ],
        &top_hits,
    );

    let query = q("foo", 3.0, 7, 5)?;
    let top_hits = top(&searcher, &query, 2)?;
    assert_equals(&2, &top_hits.len());
    CheckHits::check_explanations(&query, Some(""), &searcher)?;
    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(4, (3f32 as f64 * (5. / (5. + 0.))) as f32),
            ScoreDoc::new(2, (3f32 as f64 * (5. / (5. + 1.))) as f32),
        ],
        &top_hits,
    );

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// Port of `testOverUnderFlow`.
fn test_over_under_flow() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut doc = field_doc();

    for v in [3, 12, -10, i64::MAX, i64::MIN] {
        set_long_value(&mut doc, v);
        w.add_document(&doc)?;
    }

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = q("foo", 3.0, i64::MAX - 1, 100)?;
    let collector_manager = TopScoreDocCollectorManager::new(2, None, 1)?;
    let top_hits = searcher
        .search_with_manager(Arc::clone(&query), &collector_manager)?
        .score_docs;
    assert_equals(&2, &top_hits.len());
    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(3, (3f32 as f64 * (100. / (100. + 1.))) as f32),
            // rounding makes the distance treated as if it was MAX_VALUE
            ScoreDoc::new(0, (3f32 as f64 * (100. / (100. + i64::MAX as f64))) as f32),
        ],
        &top_hits,
    );

    let query = q("foo", 3.0, i64::MIN + 1, 100)?;
    // Java reuses the same `collectorManager`.
    let top_hits = searcher
        .search_with_manager(Arc::clone(&query), &collector_manager)?
        .score_docs;
    assert_equals(&2, &top_hits.len());
    CheckHits::check_explanations(&query, Some(""), &searcher)?;
    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(4, (3f32 as f64 * (100. / (100. + 1.))) as f32),
            // rounding makes the distance treated as if it was MAX_VALUE
            ScoreDoc::new(0, (3f32 as f64 * (100. / (100. + i64::MAX as f64))) as f32),
        ],
        &top_hits,
    );

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// Port of `testMissingField`.
fn test_missing_field() -> Result<()> {
    let reader = Arc::new(MultiReader::new(Vec::new(), true)?);
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query = q("foo", 3.0, 10, 5)?;
    let top_hits = searcher.search(query, 2)?;
    assert_equals(&0i64, &top_hits.total_hits.value());
    Ok(())
}

/// Port of `testMissingValue`.
fn test_missing_value() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut doc = field_doc();

    set_long_value(&mut doc, 3);
    w.add_document(&doc)?;

    w.add_document(&Document::new())?;

    set_long_value(&mut doc, 7);
    w.add_document(&doc)?;

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = q("foo", 3.0, 10, 5)?;
    let top_hits = top(&searcher, &query, 3)?;
    assert_equals(&2, &top_hits.len());
    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(2, (3f32 as f64 * (5. / (5. + 3.))) as f32),
            ScoreDoc::new(0, (3f32 as f64 * (5. / (5. + 7.))) as f32),
        ],
        &top_hits,
    );

    CheckHits::check_explanations(&query, Some(""), &searcher)?;

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// Port of `testMultiValued`.
fn test_multi_valued() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    w.add_document(&doc(&[3, 1000, i64::MAX]))?;
    w.add_document(&doc(&[-100, 12, 999]))?;
    w.add_document(&doc(&[i64::MIN, -1000, 8]))?;
    w.add_document(&doc(&[-1]))?;
    w.add_document(&doc(&[i64::MIN, 7]))?;

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = q("foo", 3.0, 10, 5)?;
    let top_hits = top(&searcher, &query, 2)?;
    assert_equals(&2, &top_hits.len());
    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(1, (3f32 as f64 * (5. / (5. + 2.))) as f32),
            ScoreDoc::new(2, (3f32 as f64 * (5. / (5. + 2.))) as f32),
        ],
        &top_hits,
    );

    let query = q("foo", 3.0, 7, 5)?;
    let top_hits = top(&searcher, &query, 2)?;
    assert_equals(&2, &top_hits.len());
    CheckHits::check_explanations(&query, Some(""), &searcher)?;
    CheckHits::check_equal(
        query.as_ref(),
        &[
            ScoreDoc::new(4, (3f32 as f64 * (5. / (5. + 0.))) as f32),
            ScoreDoc::new(2, (3f32 as f64 * (5. / (5. + 1.))) as f32),
        ],
        &top_hits,
    );

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// Port of `testRandom`.
fn test_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    )))?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    let mut doc = field_doc();

    let num_docs = LuceneTestCase::at_least(10000);
    for _i in 0..num_docs {
        let v = random().next_long();
        set_long_value(&mut doc, v);
        w.add_document(&doc)?;
    }

    let reader = rucene::index::directory_reader::open_with_writer(&*w)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    for _iter in 0..10 {
        let origin = random().next_long();
        let mut pivot_distance;
        loop {
            pivot_distance = random().next_long();
            if pivot_distance > 0 {
                break;
            }
        }
        let boost = (1 + random().next_int_bound(10)) as f32 / 3.0;
        let query = q("foo", boost, origin, pivot_distance)?;
        CheckHits::check_top_scores(&*random(), &query, &searcher)?;
    }

    (reader as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}
