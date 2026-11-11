//! Port of `lucene/core/src/test/org/apache/lucene/search/comparators/TestUpdateableDocIdSetIterator.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::comparators::updateable_doc_id_set_iterator::UpdateableDocIdSetIterator;
use rucene::search::{range, DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::FixedBitSet;

use crate::core_support::{assert_equals, class, test};

/// `TestUpdateableDocIdSetIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.comparators.TestUpdateableDocIdSetIterator",
        vec![
            test("testNextDoc", test_next_doc),
            test("testAdvance", test_advance),
            test("testIntoBitSet", test_into_bit_set),
            test("testDocIDRunEnd", test_doc_id_run_end),
        ],
    )
}

/// `DocIdSetIterator.range(min, max)`, boxed.
fn r(min: i32, max: i32) -> Result<Box<dyn DocIdSetIterator>> {
    Ok(Box::new(range(min, max)?))
}

fn test_next_doc() -> Result<()> {
    let mut iterator = UpdateableDocIdSetIterator::new();
    iterator.update(r(10, 20)?);
    assert_equals(&10, &iterator.next_doc()?);
    iterator.update(r(10, 12)?);
    assert_equals(&11, &iterator.next_doc()?);
    let mut in_ = r(10, 15)?;
    in_.advance(14)?;
    iterator.update(in_);
    assert_equals(&14, &iterator.next_doc()?);
    assert_equals(&NO_MORE_DOCS, &iterator.next_doc()?);
    Ok(())
}

fn test_advance() -> Result<()> {
    let mut iterator = UpdateableDocIdSetIterator::new();
    iterator.update(r(10, 20)?);
    assert_equals(&12, &iterator.advance(12)?);
    iterator.update(r(10, 15)?);
    assert_equals(&13, &iterator.advance(13)?);
    let mut in_ = r(10, 20)?;
    in_.advance(15)?;
    iterator.update(in_);
    assert_equals(&15, &iterator.advance(15)?);
    assert_equals(&NO_MORE_DOCS, &iterator.advance(20)?);
    Ok(())
}

fn test_into_bit_set() -> Result<()> {
    let mut iterator = UpdateableDocIdSetIterator::new();
    iterator.update(r(10, 18)?);
    assert_equals(&10, &iterator.next_doc()?);
    let mut bit_set = FixedBitSet::new(100);
    iterator.into_bit_set(15, &mut bit_set, 5)?;
    assert_equals(&15, &iterator.doc_id());
    let mut expected = FixedBitSet::new(100);
    expected.set_range(5, 10);
    assert_equals(&expected, &bit_set);

    iterator.update(r(12, 25)?);
    bit_set.clear_all();
    iterator.into_bit_set(20, &mut bit_set, 8)?;
    assert_equals(&20, &iterator.doc_id());
    expected.clear_all();
    expected.set_range(7, 12);
    assert_equals(&expected, &bit_set);

    let mut in_ = r(15, 25)?;
    in_.advance(23)?;
    iterator.update(in_);
    bit_set.clear_all();
    iterator.into_bit_set(30, &mut bit_set, 10)?;
    assert_equals(&NO_MORE_DOCS, &iterator.doc_id());
    expected.clear_all();
    expected.set_range(13, 15);
    assert_equals(&expected, &bit_set);
    Ok(())
}

fn test_doc_id_run_end() -> Result<()> {
    let mut iterator = UpdateableDocIdSetIterator::new();
    iterator.update(r(10, 18)?);
    assert_equals(&10, &iterator.next_doc()?);
    assert_equals(&18, &iterator.doc_id_run_end()?);
    assert_equals(&10, &iterator.doc_id()); // no side effect

    iterator.update(r(8, 25)?);
    assert_equals(&25, &iterator.doc_id_run_end()?);
    assert_equals(&10, &iterator.doc_id()); // no side effect

    let mut in_ = r(5, 25)?;
    in_.advance(12)?;
    iterator.update(in_);
    assert_equals(&11, &iterator.doc_id_run_end()?);
    assert_equals(&10, &iterator.doc_id()); // no side effect
    assert_equals(&12, &iterator.next_doc()?);
    Ok(())
}
