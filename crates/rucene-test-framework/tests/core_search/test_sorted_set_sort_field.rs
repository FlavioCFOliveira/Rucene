//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSortedSetSortField.java`.
//!
//! Simple tests for SortedSetSortField, indexing the sortedset up front

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::multi_reader::MultiReader;
use rucene::index::Term;
use rucene::search::sort::{MissingValue, Sort, SortField};
use rucene::search::sorted_set_selector::SortedSetSelectorType;
use rucene::search::sorted_set_sort_field::SortedSetSortField;
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::test_sorted_set_selector::{check_sorted, id, k};
use crate::core_support::{assert_equals, class, test};

/// `TestSortedSetSortField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSortedSetSortField",
        vec![
            test("testEmptyIndex", test_empty_index),
            test("testEquals", test_equals),
            test("testForward", test_forward),
            test("testReverse", test_reverse),
            test("testMissingFirst", test_missing_first),
            test("testMissingLast", test_missing_last),
            test("testSingleton", test_singleton),
        ],
    )
}

/// `SortedSetSelector.Type.values()`.
const SELECTOR_TYPES: [SortedSetSelectorType; 4] = [
    SortedSetSelectorType::MIN,
    SortedSetSelectorType::MAX,
    SortedSetSelectorType::MIDDLE_MIN,
    SortedSetSelectorType::MIDDLE_MAX,
];

/// `new SortedSetSortField(field, reverse)`.
fn ssf(field: &str, reverse: bool) -> Result<SortField> {
    Ok(SortedSetSortField::new(field, reverse)?.into_sort_field())
}

/// Port of `TestSortedSetSortField.testEmptyIndex`.
fn test_empty_index() -> Result<()> {
    let empty = LuceneTestCase::new_searcher(Arc::new(MultiReader::new(Vec::new(), true)?));
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("contents", "foo")));
    let td = empty.search_with_sort_and_scores(
        Arc::clone(&query),
        10,
        &Sort::new_fields(vec![ssf("sortedset", false)?])?,
        true,
    )?;
    assert_equals(&0, &td.total_hits.value());

    // for an empty index, any selector should work
    for v in SELECTOR_TYPES {
        let td = empty.search_with_sort_and_scores(
            Arc::clone(&query),
            10,
            &Sort::new_fields(vec![SortedSetSortField::with_selector(
                "sortedset",
                false,
                v,
            )?
            .into_sort_field()])?,
            true,
        )?;
        assert_equals(&0, &td.total_hits.value());
    }
    Ok(())
}

/// `sf.equals(other)`: `other` is `null` (`None`) or any object.
fn java_equals(sf: &SortField, other: Option<&dyn Any>) -> bool {
    other
        .and_then(|o| o.downcast_ref::<SortField>())
        .is_some_and(|o| sf == o)
}

/// `sf.hashCode()`.
fn hash_code(sf: &SortField) -> u64 {
    let mut hasher = DefaultHasher::new();
    sf.hash(&mut hasher);
    hasher.finish()
}

/// Port of `TestSortedSetSortField.testEquals`.
fn test_equals() -> Result<()> {
    let sf = ssf("a", false)?;
    Assert::assert_false(java_equals(&sf, None));

    assert_equals(&sf, &sf);

    let sf2 = ssf("a", false)?;
    assert_equals(&sf, &sf2);
    assert_equals(&hash_code(&sf), &hash_code(&sf2));

    Assert::assert_false(java_equals(&sf, Some(&ssf("a", true)?)));
    Assert::assert_false(java_equals(&sf, Some(&ssf("b", false)?)));
    Assert::assert_false(java_equals(
        &sf,
        Some(
            &SortedSetSortField::with_selector("a", false, SortedSetSelectorType::MAX)?
                .into_sort_field(),
        ),
    ));
    Assert::assert_false(java_equals(&sf, Some(&"foo")));
    Ok(())
}

/// Port of `TestSortedSetSortField.testForward`.
fn test_forward() -> Result<()> {
    // 'bar' comes before 'baz'
    check_sorted(
        &[vec![k("baz"), id("2")], vec![k("foo"), k("bar"), id("1")]],
        true,
        SortedSetSortField::with_missing_value("value", false, SortedSetSelectorType::MIN, None)?
            .into_sort_field(),
        2,
        &["1", "2"],
    )
}

/// Port of `TestSortedSetSortField.testReverse`.
fn test_reverse() -> Result<()> {
    // 'bar' comes before 'baz'
    check_sorted(
        &[vec![k("foo"), k("bar"), id("1")], vec![k("baz"), id("2")]],
        true,
        SortedSetSortField::with_missing_value("value", true, SortedSetSelectorType::MIN, None)?
            .into_sort_field(),
        2,
        &["2", "1"],
    )
}

/// Port of `TestSortedSetSortField.testMissingFirst`.
fn test_missing_first() -> Result<()> {
    // 'bar' comes before 'baz'
    // null comes first
    check_sorted(
        &[
            vec![k("baz"), id("2")],
            vec![k("foo"), k("bar"), id("1")],
            vec![id("3")],
        ],
        true,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIN,
            Some(MissingValue::StringFirst),
        )?
        .into_sort_field(),
        3,
        &["3", "1", "2"],
    )
}

/// Port of `TestSortedSetSortField.testMissingLast`.
fn test_missing_last() -> Result<()> {
    // 'bar' comes before 'baz'
    // null comes last
    check_sorted(
        &[
            vec![k("baz"), id("2")],
            vec![k("foo"), k("bar"), id("1")],
            vec![id("3")],
        ],
        true,
        SortedSetSortField::with_missing_value(
            "value",
            false,
            SortedSetSelectorType::MIN,
            Some(MissingValue::StringLast),
        )?
        .into_sort_field(),
        3,
        &["1", "2", "3"],
    )
}

/// Port of `TestSortedSetSortField.testSingleton`.
fn test_singleton() -> Result<()> {
    // 'bar' comes before 'baz'
    check_sorted(
        &[vec![k("baz"), id("2")], vec![k("bar"), id("1")]],
        true,
        SortedSetSortField::with_missing_value("value", false, SortedSetSelectorType::MIN, None)?
            .into_sort_field(),
        2,
        &["1", "2"],
    )
}
