//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocsWithFieldSet.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::DocsWithFieldSet;
use rucene::error::Result;
use rucene::search::NO_MORE_DOCS;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::TestUtil;

use crate::core_support::{assert_equals, class, random, test, IAE};

/// `TestDocsWithFieldSet`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocsWithFieldSet",
        vec![
            test("testDense", test_dense),
            test("testSparse", test_sparse),
            test("testAddRange", test_add_range),
            test("testAddRangeEmpty", test_add_range_empty),
            test(
                "testAddRangeRejectsInverted",
                test_add_range_rejects_inverted,
            ),
            test("testDenseThenSparse", test_dense_then_sparse),
        ],
    )
}

/// The documents `set.iterator()` returns, then `NO_MORE_DOCS`.
fn assert_iterates(set: &DocsWithFieldSet, expected: impl IntoIterator<Item = i32>) -> Result<()> {
    let mut it = set.iterator()?;
    for doc in expected {
        assert_equals(&doc, &it.next_doc()?);
    }
    assert_equals(&NO_MORE_DOCS, &it.next_doc()?);
    Ok(())
}

/// Port of `TestDocsWithFieldSet.testDense`.
fn test_dense() -> Result<()> {
    let mut set = DocsWithFieldSet::new();
    assert_iterates(&set, [])?;

    set.add(0)?;
    assert_iterates(&set, [0])?;

    let ram_bytes_used = set.ram_bytes_used();
    for i in 1..1000 {
        set.add(i)?;
    }
    assert_equals(&ram_bytes_used, &set.ram_bytes_used());
    assert_iterates(&set, 0..1000)
}

/// Port of `TestDocsWithFieldSet.testSparse`.
fn test_sparse() -> Result<()> {
    let mut set = DocsWithFieldSet::new();
    let doc = random().next_int_bound(10000);
    set.add(doc)?;
    assert_iterates(&set, [doc])?;
    let doc2 = doc + TestUtil::next_int(&*random(), 1, 100);
    set.add(doc2)?;
    assert_iterates(&set, [doc, doc2])
}

/// Port of `TestDocsWithFieldSet.testAddRange`.
fn test_add_range() -> Result<()> {
    let mut set = DocsWithFieldSet::new();
    set.add_range(0, 5)?;
    assert_equals(&5, &set.cardinality());
    assert_iterates(&set, 0..5)?;

    set.add_range(10, 13)?;
    assert_equals(&8, &set.cardinality());
    assert_iterates(&set, (0..5).chain(10..13))
}

/// Empty ranges must be no-ops and must not regress lastDocId.
///
/// Port of `TestDocsWithFieldSet.testAddRangeEmpty`.
fn test_add_range_empty() -> Result<()> {
    let mut set = DocsWithFieldSet::new();
    set.add_range(0, 0)?;
    assert_equals(&0, &set.cardinality());

    set.add_range(0, 5)?;
    assert_equals(&5, &set.cardinality());

    // Empty range starting past lastDocId must not regress state, and a subsequent add must still
    // be ordered relative to the original lastDocId (4), not the empty range's nominal end.
    set.add_range(10, 10)?;
    assert_equals(&5, &set.cardinality());
    set.add(5)?;
    assert_equals(&6, &set.cardinality());

    assert_iterates(&set, 0..6)
}

/// Port of `TestDocsWithFieldSet.testAddRangeRejectsInverted`.
fn test_add_range_rejects_inverted() -> Result<()> {
    let mut set = DocsWithFieldSet::new();
    expect_throws(IAE, || set.add_range(5, 3));
    Ok(())
}

/// Port of `TestDocsWithFieldSet.testDenseThenSparse`.
fn test_dense_then_sparse() -> Result<()> {
    let dense_count = random().next_int_bound(10000);
    let next_doc = dense_count + random().next_int_bound(10000);
    let mut set = DocsWithFieldSet::new();
    for i in 0..dense_count {
        set.add(i)?;
    }
    set.add(next_doc)?;
    assert_iterates(&set, (0..dense_count).chain([next_doc]))
}
