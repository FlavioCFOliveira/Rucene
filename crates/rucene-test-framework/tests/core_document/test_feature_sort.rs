//! Port of `lucene/core/src/test/org/apache/lucene/document/TestFeatureSort.java`:
//! sorting using a feature from a `FeatureField`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::feature_field::{decode_feature_value, FeatureField, MAX_FREQ};
use rucene::document::{Document, NumericDocValuesField, Store};
use rucene::error::Result;
use rucene::index::{DirectoryReader, IndexReader};
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::{MatchAllDocsQuery, Query, ScoreDoc, TopFieldDocs};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::search::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_null, class, random, test};

/// `TestFeatureSort`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestFeatureSort",
        vec![
            test("testFeature", test_feature),
            test("testFeatureMissing", test_feature_missing),
            test(
                "testFeatureMissingFieldInSegment",
                test_feature_missing_field_in_segment,
            ),
            test(
                "testFeatureMissingFeatureNameInSegment",
                test_feature_missing_feature_name_in_segment,
            ),
            test("testFeatureMultipleMissing", test_feature_multiple_missing),
            test("testDuelFloat", test_duel_float),
        ],
    )
}

/// `newIndexWriterConfig().setMergePolicy(newLogMergePolicy(random().nextBoolean()))`
/// and `new RandomIndexWriter(random(), dir, config)`.
fn writer(dir: &TestDirectory) -> Result<RandomIndexWriter> {
    let config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    )))?;
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)
}

/// A document of `FeatureField(field, name, value)` and
/// `newStringField("value", text, Store.YES)`, or an empty one.
fn doc(feature: Option<(&str, f32, Option<&str>)>) -> Result<Document> {
    let mut doc = Document::new();
    if let Some((name, value, text)) = feature {
        doc.add(Box::new(FeatureField::new("field", name, value)?));
        if let Some(text) = text {
            doc.add(Box::new(LuceneTestCase::new_string_field(
                "value",
                text,
                Store::YES,
            )));
        }
    }
    Ok(doc)
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

/// `searcher.search(MatchAllDocsQuery.INSTANCE, 10, new Sort(FeatureField.newFeatureSort("field", "name")))`
/// over `ir`, then the stored `value` of each hit.
fn sorted_values(
    ir: &Arc<dyn DirectoryReader>,
    expected_total: i64,
) -> Result<Vec<Option<String>>> {
    let searcher = LuceneTestCase::new_searcher(Arc::clone(ir));
    let stored_fields = searcher.stored_fields()?;
    let sort = Sort::new_fields(vec![FeatureField::new_feature_sort("field", "name")])?;
    let td = searcher.search_with_sort(match_all(), 10, &sort)?;
    assert_equals(&expected_total, &td.total_hits.value());
    let mut out = Vec::new();
    for sd in &td.score_docs {
        out.push(stored_fields.document(sd.score_doc.doc)?.get("value"));
    }
    Ok(out)
}

/// Port of `testFeature`.
fn test_feature() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    writer.add_document(&doc(Some(("name", 30.1, Some("30.1"))))?)?;
    writer.add_document(&doc(Some(("name", 1.3, Some("1.3"))))?)?;
    writer.add_document(&doc(Some(("name", 4.2, Some("4.2"))))?)?;
    let ir = writer.get_reader()?;
    writer.close()?;

    let v = sorted_values(&ir, 3)?;
    // numeric order
    assert_equals(&Some("30.1".to_string()), &v[0]);
    assert_equals(&Some("4.2".to_string()), &v[1]);
    assert_equals(&Some("1.3".to_string()), &v[2]);

    (ir as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// The shared assertions of the "missing" tests: `null is treated as 0`.
fn assert_missing_order(v: &[Option<String>]) {
    assert_equals(&Some("4.2".to_string()), &v[0]);
    assert_equals(&Some("1.3".to_string()), &v[1]);
    for value in &v[2..] {
        assert_null(value);
    }
}

/// Port of `testFeatureMissing`.
fn test_feature_missing() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    writer.add_document(&doc(None)?)?;
    writer.add_document(&doc(Some(("name", 1.3, Some("1.3"))))?)?;
    writer.add_document(&doc(Some(("name", 4.2, Some("4.2"))))?)?;
    let ir = writer.get_reader()?;
    writer.close()?;

    // null is treated as 0
    assert_missing_order(&sorted_values(&ir, 3)?);

    (ir as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testFeatureMissingFieldInSegment`.
fn test_feature_missing_field_in_segment() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    writer.add_document(&doc(None)?)?;
    writer.commit()?;
    writer.add_document(&doc(Some(("name", 1.3, Some("1.3"))))?)?;
    writer.add_document(&doc(Some(("name", 4.2, Some("4.2"))))?)?;
    let ir = writer.get_reader()?;
    writer.close()?;

    // null is treated as 0
    assert_missing_order(&sorted_values(&ir, 3)?);

    (ir as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testFeatureMissingFeatureNameInSegment`.
fn test_feature_missing_feature_name_in_segment() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    writer.add_document(&doc(Some(("different_name", 0.5, None)))?)?;
    writer.commit()?;
    writer.add_document(&doc(Some(("name", 1.3, Some("1.3"))))?)?;
    writer.add_document(&doc(Some(("name", 4.2, Some("4.2"))))?)?;
    let ir = writer.get_reader()?;
    writer.close()?;

    // null is treated as 0
    assert_missing_order(&sorted_values(&ir, 3)?);

    (ir as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testFeatureMultipleMissing`.
fn test_feature_multiple_missing() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;
    for _ in 0..5 {
        writer.add_document(&doc(None)?)?;
    }
    writer.add_document(&doc(Some(("name", 1.3, Some("1.3"))))?)?;
    writer.add_document(&doc(Some(("name", 4.2, Some("4.2"))))?)?;
    let ir = writer.get_reader()?;
    writer.close()?;

    // null is treated as 0
    assert_missing_order(&sorted_values(&ir, 7)?);

    (ir as Arc<dyn IndexReader>).close()?;
    dir.close()
}

fn score_docs(td: &TopFieldDocs) -> Vec<ScoreDoc> {
    td.score_docs
        .iter()
        .map(|fd| fd.score_doc.clone())
        .collect()
}

/// Port of `testDuelFloat`: this duel gives `compareBottom` and
/// `compareTop` some coverage.
fn test_duel_float() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = LuceneTestCase::at_least(100);
    for _d in 0..num_docs {
        let mut doc = Document::new();
        if random().next_boolean() {
            let mut f;
            loop {
                let freq = TestUtil::next_int(&*random(), 1, MAX_FREQ);
                f = decode_feature_value(freq as f32);
                if f >= f32::MIN_POSITIVE {
                    break;
                }
            }
            doc.add(Box::new(NumericDocValuesField::new(
                "float",
                f.to_bits() as i32 as i64,
            )));
            doc.add(Box::new(FeatureField::new("feature", "foo", f)?));
        }
        w.add_document(&doc)?;
    }
    let r = w.get_reader()?;
    w.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    let float_sort = || -> Result<Sort> {
        Sort::new_fields(vec![SortField::new_reverse(
            Some("float".to_string()),
            SortFieldType::Float,
            true,
        )?])
    };
    let feature_sort = || -> Result<Sort> {
        Sort::new_fields(vec![FeatureField::new_feature_sort("feature", "foo")])
    };
    let mut top_docs: Option<TopFieldDocs> = None;
    let mut feature_top_docs: Option<TopFieldDocs> = None;
    loop {
        match (&top_docs, &feature_top_docs) {
            (Some(td), Some(ftd)) => {
                // `searcher.searchAfter(topDocs.scoreDocs[length - 1], …)`: the
                // last hit is a `FieldDoc`.
                let after = td.score_docs.last().cloned();
                let feature_after = ftd.score_docs.last().cloned();
                top_docs = Some(searcher.search_after_with_sort(
                    after,
                    match_all(),
                    10,
                    &float_sort()?,
                )?);
                feature_top_docs = Some(searcher.search_after_with_sort(
                    feature_after,
                    match_all(),
                    10,
                    &feature_sort()?,
                )?);
            }
            _ => {
                top_docs = Some(searcher.search_with_sort(match_all(), 10, &float_sort()?)?);
                feature_top_docs =
                    Some(searcher.search_with_sort(match_all(), 10, &feature_sort()?)?);
            }
        }
        let td = top_docs.as_ref().expect("topDocs");
        let ftd = feature_top_docs.as_ref().expect("featureTopDocs");
        CheckHits::check_equal(
            &MatchAllDocsQuery::instance(),
            &score_docs(td),
            &score_docs(ftd),
        );
        if td.score_docs.is_empty() {
            break;
        }
    }

    (r as Arc<dyn IndexReader>).close()?;
    dir.close()
}
