//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSortedNumericSortField.java`.
//!
//! Simple tests for SortedNumericSortField

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, DoubleField, FloatField, IntField, Store};
use rucene::error::Result;
use rucene::index::multi_reader::MultiReader;
use rucene::index::Term;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::sort::{MissingValue, Sort, SortField, SortFieldType};
use rucene::search::sorted_numeric_selector::SortedNumericSelectorType;
use rucene::search::sorted_numeric_sort_field::SortedNumericSortField;
use rucene::search::term_query::TermQuery;
use rucene::search::{MatchAllDocsQuery, Query};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestSortedNumericSortField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSortedNumericSortField",
        vec![
            test("testEmptyIndex", test_empty_index),
            test("testEquals", test_equals),
            test("testForward", test_forward),
            test("testReverse", test_reverse),
            test("testMissingFirst", test_missing_first),
            test("testMissingLast", test_missing_last),
            test("testSingleton", test_singleton),
            test("testFloat", test_float),
            test("testDouble", test_double),
        ],
    )
}

/// `SortedNumericSelector.Type.values()`.
const SELECTOR_TYPES: [SortedNumericSelectorType; 2] = [
    SortedNumericSelectorType::MIN,
    SortedNumericSelectorType::MAX,
];

fn snsf(field: &str, t: SortFieldType) -> Result<SortField> {
    Ok(SortedNumericSortField::new(field, t)?.into_sort_field())
}

/// Port of `TestSortedNumericSortField.testEmptyIndex`.
fn test_empty_index() -> Result<()> {
    let empty = LuceneTestCase::new_searcher(Arc::new(MultiReader::new(Vec::new(), true)?));
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("contents", "foo")));

    let td = empty.search_with_sort_and_scores(
        Arc::clone(&query),
        10,
        &Sort::new_fields(vec![snsf("sortednumeric", SortFieldType::Long)?])?,
        true,
    )?;
    assert_equals(&0, &td.total_hits.value());

    // for an empty index, any selector should work
    for v in SELECTOR_TYPES {
        let td = empty.search_with_sort_and_scores(
            Arc::clone(&query),
            10,
            &Sort::new_fields(vec![SortedNumericSortField::with_selector(
                "sortednumeric",
                SortFieldType::Long,
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

/// Port of `TestSortedNumericSortField.testEquals`.
fn test_equals() -> Result<()> {
    let sf = snsf("a", SortFieldType::Long)?;
    Assert::assert_false(java_equals(&sf, None));

    assert_equals(&sf, &sf);

    let sf2 = snsf("a", SortFieldType::Long)?;
    assert_equals(&sf, &sf2);
    assert_equals(&hash_code(&sf), &hash_code(&sf2));

    Assert::assert_false(java_equals(
        &sf,
        Some(
            &SortedNumericSortField::with_reverse("a", SortFieldType::Long, true)?
                .into_sort_field(),
        ),
    ));
    Assert::assert_false(java_equals(&sf, Some(&snsf("a", SortFieldType::Float)?)));
    Assert::assert_false(java_equals(&sf, Some(&snsf("b", SortFieldType::Long)?)));
    Assert::assert_false(java_equals(
        &sf,
        Some(
            &SortedNumericSortField::with_selector(
                "a",
                SortFieldType::Long,
                false,
                SortedNumericSelectorType::MAX,
            )?
            .into_sort_field(),
        ),
    ));
    Assert::assert_false(java_equals(&sf, Some(&"foo")));
    Ok(())
}

/// `newStringField("id", id, Field.Store.YES)`.
fn id_field(id: &str) -> Box<rucene::document::Field> {
    Box::new(LuceneTestCase::new_string_field("id", id, Store::YES))
}

/// `searcher.storedFields().document(doc).get("id")`.
fn id_of(searcher: &IndexSearcher, doc: i32) -> Result<Option<String>> {
    Ok(searcher
        .stored_fields()?
        .document(doc)?
        .get("id")
        .map(|s| s.to_string()))
}

/// `assertEquals(expected, searcher.storedFields().document(doc).get("id"))`.
fn assert_id(searcher: &IndexSearcher, expected: &str, doc: i32) -> Result<()> {
    assert_equals(&Some(expected.to_string()), &id_of(searcher, doc)?);
    Ok(())
}

/// A document builder, run where Java builds the document: between the
/// writer's creation and the previous `addDocument`, so that the random draws
/// of `newStringField` keep their place.
type DocFn = Box<dyn Fn() -> Document>;

/// The index, sort and assertions every sorting test shares: `docs` are
/// built and added in order, `sort_field` sorts all of them, `expected_ids`
/// are the ids in the sorted order.
fn check_sorted(docs: Vec<DocFn>, sort_field: SortField, expected_ids: &[&str]) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for doc in &docs {
        writer.add_document(&doc())?;
    }
    let ir = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&ir));
    let sort = Sort::new_fields(vec![sort_field])?;

    let td = searcher.search_with_sort(Arc::new(MatchAllDocsQuery::instance()), 10, &sort)?;
    assert_equals(&(expected_ids.len() as i64), &td.total_hits.value());
    for (i, id) in expected_ids.iter().enumerate() {
        assert_id(&searcher, id, td.score_docs[i].score_doc.doc)?;
    }

    drop(searcher);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// A document with an `IntField("value", v, NO)` per value and the `id`.
fn int_doc(values: &[i32], id: &str) -> Document {
    let mut doc = Document::new();
    for &v in values {
        doc.add(Box::new(IntField::new("value", v, Store::NO)));
    }
    doc.add(id_field(id));
    doc
}

/// Port of `TestSortedNumericSortField.testForward`.
fn test_forward() -> Result<()> {
    // 3 comes before 5
    check_sorted(
        vec![
            Box::new(|| int_doc(&[5], "2")),
            Box::new(|| int_doc(&[3, 7], "1")),
        ],
        snsf("value", SortFieldType::Int)?,
        &["1", "2"],
    )
}

/// Port of `TestSortedNumericSortField.testReverse`.
fn test_reverse() -> Result<()> {
    // 'bar' comes before 'baz'
    check_sorted(
        vec![
            Box::new(|| int_doc(&[3, 7], "1")),
            Box::new(|| int_doc(&[5], "2")),
        ],
        SortedNumericSortField::with_reverse("value", SortFieldType::Int, true)?.into_sort_field(),
        &["2", "1"],
    )
}

/// Port of `TestSortedNumericSortField.testMissingFirst`.
fn test_missing_first() -> Result<()> {
    let sort_field = SortedNumericSortField::with_missing_value(
        "value",
        SortFieldType::Int,
        false,
        SortedNumericSelectorType::MIN,
        Some(MissingValue::Int(i32::MIN)),
    )?
    .into_sort_field();
    // 3 comes before 5
    // null comes first
    check_sorted(
        vec![
            Box::new(|| int_doc(&[5], "2")),
            Box::new(|| int_doc(&[3, 7], "1")),
            Box::new(|| int_doc(&[], "3")),
        ],
        sort_field,
        &["3", "1", "2"],
    )
}

/// Port of `TestSortedNumericSortField.testMissingLast`.
fn test_missing_last() -> Result<()> {
    let sort_field = SortedNumericSortField::with_missing_value(
        "value",
        SortFieldType::Int,
        false,
        SortedNumericSelectorType::MIN,
        Some(MissingValue::Int(i32::MAX)),
    )?
    .into_sort_field();
    // 3 comes before 5
    // null comes last
    check_sorted(
        vec![
            Box::new(|| int_doc(&[5], "2")),
            Box::new(|| int_doc(&[3, 7], "1")),
            Box::new(|| int_doc(&[], "3")),
        ],
        sort_field,
        &["1", "2", "3"],
    )
}

/// Port of `TestSortedNumericSortField.testSingleton`.
fn test_singleton() -> Result<()> {
    // 3 comes before 5
    check_sorted(
        vec![
            Box::new(|| int_doc(&[5], "2")),
            Box::new(|| int_doc(&[3], "1")),
        ],
        snsf("value", SortFieldType::Int)?,
        &["1", "2"],
    )
}

/// Port of `TestSortedNumericSortField.testFloat`.
fn test_float() -> Result<()> {
    let d1: DocFn = Box::new(|| {
        let mut d1 = Document::new();
        d1.add(Box::new(FloatField::new("value", -3.0, Store::NO)));
        d1.add(id_field("2"));
        d1
    });
    let d2: DocFn = Box::new(|| {
        let mut d2 = Document::new();
        d2.add(Box::new(FloatField::new("value", -5.0, Store::NO)));
        d2.add(Box::new(FloatField::new("value", 7.0, Store::NO)));
        d2.add(id_field("1"));
        d2
    });
    // -5 comes before -3
    check_sorted(
        vec![d1, d2],
        snsf("value", SortFieldType::Float)?,
        &["1", "2"],
    )
}

/// Port of `TestSortedNumericSortField.testDouble`.
fn test_double() -> Result<()> {
    let d1: DocFn = Box::new(|| {
        let mut d1 = Document::new();
        d1.add(Box::new(DoubleField::new("value", -3.0, Store::YES)));
        d1.add(id_field("2"));
        d1
    });
    let d2: DocFn = Box::new(|| {
        let mut d2 = Document::new();
        d2.add(Box::new(DoubleField::new("value", -5.0, Store::YES)));
        d2.add(Box::new(DoubleField::new("value", 7.0, Store::YES)));
        d2.add(id_field("1"));
        d2
    });
    // -5 comes before -3
    check_sorted(
        vec![d1, d2],
        snsf("value", SortFieldType::Double)?,
        &["1", "2"],
    )
}
