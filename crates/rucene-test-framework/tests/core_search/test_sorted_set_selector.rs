//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSortedSetSelector.java`.
//!
//! Tests for SortedSetSortField selectors other than MIN, these require optional codec support
//! (random access to ordinals)

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, KeywordField, SortedSetDocValuesField, Store};
use rucene::error::Result;
use rucene::search::sort::{MissingValue, Sort, SortField};
use rucene::search::sorted_set_selector::SortedSetSelectorType;
use rucene::search::sorted_set_sort_field::SortedSetSortField;
use rucene::search::MatchAllDocsQuery;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::SuppressCodecs;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, random, test};

/// `TestSortedSetSelector`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.search.TestSortedSetSelector",
        vec![
            test("testMax", test_max),
            test("testMaxReverse", test_max_reverse),
            test("testMaxMissingFirst", test_max_missing_first),
            test("testMaxMissingLast", test_max_missing_last),
            test("testMaxSingleton", test_max_singleton),
            test("testMiddleMin", test_middle_min),
            test("testMiddleMinReverse", test_middle_min_reverse),
            test("testMiddleMinMissingFirst", test_middle_min_missing_first),
            test("testMiddleMinMissingLast", test_middle_min_missing_last),
            test("testMiddleMinSingleton", test_middle_min_singleton),
            test("testMiddleMax", test_middle_max),
            test("testMiddleMaxReverse", test_middle_max_reverse),
            test("testMiddleMaxMissingFirst", test_middle_max_missing_first),
            test("testMiddleMaxMissingLast", test_middle_max_missing_last),
            test("testMiddleMaxSingleton", test_middle_max_singleton),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// One field of a document the sorting tests index, built where Java builds
/// it so that the random draws of `newBytesRef` and `newStringField` keep
/// their place.
pub enum DocPart {
    /// `new SortedSetDocValuesField("value", newBytesRef(v))`.
    Value(&'static str),
    /// `new KeywordField("value", newBytesRef(v), Field.Store.NO)`.
    Keyword(&'static str),
    /// `newStringField("id", id, Field.Store.YES)`.
    Id(&'static str),
}

/// `new SortedSetDocValuesField("value", newBytesRef(v))`.
pub fn v(value: &'static str) -> DocPart {
    DocPart::Value(value)
}

/// `new KeywordField("value", newBytesRef(v), Field.Store.NO)`.
pub fn k(value: &'static str) -> DocPart {
    DocPart::Keyword(value)
}

/// `newStringField("id", id, Field.Store.YES)`.
pub fn id(id: &'static str) -> DocPart {
    DocPart::Id(id)
}

/// Builds the document of `parts`, in order.
fn build_doc(parts: &[DocPart]) -> Result<Document> {
    let mut doc = Document::new();
    for part in parts {
        match part {
            DocPart::Value(value) => doc.add(Box::new(SortedSetDocValuesField::new(
                "value",
                LuceneTestCase::new_bytes_ref_str(value),
            ))),
            DocPart::Keyword(value) => doc.add(Box::new(KeywordField::new_with_bytes(
                "value",
                LuceneTestCase::new_bytes_ref_str(value),
                Store::NO,
            )?)),
            DocPart::Id(id) => doc.add(Box::new(LuceneTestCase::new_string_field(
                "id",
                id,
                Store::YES,
            ))),
        }
    }
    Ok(doc)
}

/// The body every sorting test shares: index `docs` in order, search all
/// documents with `newSearcher(ir, maybe_wrap)` sorted by `sort_field`, and
/// check the total hits and the ids in sorted order.
pub fn check_sorted(
    docs: &[Vec<DocPart>],
    maybe_wrap: bool,
    sort_field: SortField,
    total_hits: i64,
    expected_ids: &[&str],
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for parts in docs {
        writer.add_document(&build_doc(parts)?)?;
    }
    let ir = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&ir), maybe_wrap);

    let sort = Sort::new_fields(vec![sort_field])?;

    let td = searcher.search_with_sort(Arc::new(MatchAllDocsQuery::instance()), 10, &sort)?;
    assert_equals(&total_hits, &td.total_hits.value());
    let stored_fields = searcher.stored_fields()?;
    for (i, expected) in expected_ids.iter().enumerate() {
        assert_equals(
            &Some(expected.to_string()),
            &stored_fields
                .document(td.score_docs[i].score_doc.doc)?
                .get("id")
                .map(|s| s.to_string()),
        );
    }

    drop(stored_fields);
    drop(searcher);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestSortedSetSelector.testMax`.
fn test_max() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'baz' comes before 'foo'
    check_sorted(
        &[vec![v("foo"), v("bar"), id("1")], vec![v("baz"), id("2")]],
        false,
        SortedSetSortField::with_missing_value("value", false, SortedSetSelectorType::MAX, None)?
            .into_sort_field(),
        2,
        &["2", "1"],
    )
}

/// Port of `TestSortedSetSelector.testMaxReverse`.
fn test_max_reverse() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'baz' comes before 'foo'
    check_sorted(
        &[vec![v("foo"), v("bar"), id("1")], vec![v("baz"), id("2")]],
        false,
        SortedSetSortField::with_missing_value("value", true, SortedSetSelectorType::MAX, None)?
            .into_sort_field(),
        2,
        &["1", "2"],
    )
}

/// Port of `TestSortedSetSelector.testMaxMissingFirst`.
fn test_max_missing_first() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // null comes first
    // 'baz' comes before 'foo'
    check_sorted(
        &[
            vec![id("1")],
            vec![v("foo"), v("bar"), id("2")],
            vec![v("baz"), id("3")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MAX,
            Some(MissingValue::StringFirst),
        )?
        .into_sort_field(),
        3,
        &["1", "3", "2"],
    )
}

/// Port of `TestSortedSetSelector.testMaxMissingLast`.
fn test_max_missing_last() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'baz' comes before 'foo'
    // null comes last
    check_sorted(
        &[
            vec![id("1")],
            vec![v("foo"), v("bar"), id("2")],
            vec![v("baz"), id("3")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MAX,
            Some(MissingValue::StringLast),
        )?
        .into_sort_field(),
        3,
        &["3", "2", "1"],
    )
}

/// Port of `TestSortedSetSelector.testMaxSingleton`.
fn test_max_singleton() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'bar' comes before 'baz'
    check_sorted(
        &[vec![v("baz"), id("2")], vec![v("bar"), id("1")]],
        false,
        SortedSetSortField::with_missing_value("value", false, SortedSetSelectorType::MAX, None)?
            .into_sort_field(),
        2,
        &["1", "2"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMin`.
fn test_middle_min() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'b' comes before 'c'
    check_sorted(
        &[
            vec![v("c"), id("2")],
            vec![v("a"), v("b"), v("c"), v("d"), id("1")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIDDLE_MIN,
            None,
        )?
        .into_sort_field(),
        2,
        &["1", "2"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMinReverse`.
fn test_middle_min_reverse() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'b' comes before 'c'
    check_sorted(
        &[
            vec![v("a"), v("b"), v("c"), v("d"), id("1")],
            vec![v("c"), id("2")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            true,
            SortedSetSelectorType::MIDDLE_MIN,
            None,
        )?
        .into_sort_field(),
        2,
        &["2", "1"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMinMissingFirst`.
fn test_middle_min_missing_first() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // null comes first
    // 'b' comes before 'c'
    check_sorted(
        &[
            vec![id("3")],
            vec![v("c"), id("2")],
            vec![v("a"), v("b"), v("c"), v("d"), id("1")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIDDLE_MIN,
            Some(MissingValue::StringFirst),
        )?
        .into_sort_field(),
        3,
        &["3", "1", "2"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMinMissingLast`.
fn test_middle_min_missing_last() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'b' comes before 'c'
    // null comes last
    check_sorted(
        &[
            vec![id("3")],
            vec![v("c"), id("2")],
            vec![v("a"), v("b"), v("c"), v("d"), id("1")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIDDLE_MIN,
            Some(MissingValue::StringLast),
        )?
        .into_sort_field(),
        3,
        &["1", "2", "3"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMinSingleton`.
fn test_middle_min_singleton() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'bar' comes before 'baz'
    check_sorted(
        &[vec![v("baz"), id("2")], vec![v("bar"), id("1")]],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIDDLE_MIN,
            None,
        )?
        .into_sort_field(),
        2,
        &["1", "2"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMax`.
fn test_middle_max() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'b' comes before 'c'
    check_sorted(
        &[
            vec![v("a"), v("b"), v("c"), v("d"), id("1")],
            vec![v("b"), id("2")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIDDLE_MAX,
            None,
        )?
        .into_sort_field(),
        2,
        &["2", "1"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMaxReverse`.
fn test_middle_max_reverse() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'b' comes before 'c'
    check_sorted(
        &[
            vec![v("b"), id("2")],
            vec![v("a"), v("b"), v("c"), v("d"), id("1")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            true,
            SortedSetSelectorType::MIDDLE_MAX,
            None,
        )?
        .into_sort_field(),
        2,
        &["1", "2"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMaxMissingFirst`.
fn test_middle_max_missing_first() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // null comes first
    // 'b' comes before 'c'
    check_sorted(
        &[
            vec![id("3")],
            vec![v("a"), v("b"), v("c"), v("d"), id("1")],
            vec![v("b"), id("2")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIDDLE_MAX,
            Some(MissingValue::StringFirst),
        )?
        .into_sort_field(),
        3,
        &["3", "2", "1"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMaxMissingLast`.
fn test_middle_max_missing_last() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'b' comes before 'c'
    // null comes last
    check_sorted(
        &[
            vec![id("3")],
            vec![v("a"), v("b"), v("c"), v("d"), id("1")],
            vec![v("b"), id("2")],
        ],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIDDLE_MAX,
            Some(MissingValue::StringLast),
        )?
        .into_sort_field(),
        3,
        &["2", "1", "3"],
    )
}

/// Port of `TestSortedSetSelector.testMiddleMaxSingleton`.
fn test_middle_max_singleton() -> Result<()> {
    // slow wrapper does not support random access ordinals (there is no need for that!)
    // 'bar' comes before 'baz'
    check_sorted(
        &[vec![v("baz"), id("2")], vec![v("bar"), id("1")]],
        false,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIDDLE_MAX,
            None,
        )?
        .into_sort_field(),
        2,
        &["1", "2"],
    )
}
