//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDocIdSetIterator.java`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::doc_id_set_iterator::{all, empty, range, DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::FixedBitSet;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_array_equals, assert_equals, class, random, test, IAE};

/// `TestDocIdSetIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDocIdSetIterator",
        vec![
            test("testEmpty", test_empty),
            test("testRangeBasic", test_range_basic),
            test("testInvalidRange", test_invalid_range),
            test("testInvalidRangeMin", test_invalid_range_min),
            test("testEmptyRange", test_empty_range),
            test("testRangeAdvance", test_range_advance),
            test("testIntoBitset", test_into_bitset),
            test("testDocIDRunEndAll", test_doc_id_run_end_all),
            test("testDocIDRunEndRange", test_doc_id_run_end_range),
        ],
    )
}

fn test_empty() -> Result<()> {
    let mut disi = empty();
    assert_equals(&-1, &disi.doc_id());
    assert_equals(&NO_MORE_DOCS, &disi.next_doc()?);
    assert_equals(&NO_MORE_DOCS, &disi.doc_id());

    let mut disi = empty();
    assert_equals(&-1, &disi.doc_id());
    assert_equals(&NO_MORE_DOCS, &disi.advance(42)?);
    assert_equals(&NO_MORE_DOCS, &disi.doc_id());
    Ok(())
}

fn test_range_basic() -> Result<()> {
    let mut disi = range(5, 8)?;
    assert_equals(&-1, &disi.doc_id());
    assert_equals(&5, &disi.next_doc()?);
    assert_equals(&6, &disi.next_doc()?);
    assert_equals(&7, &disi.next_doc()?);
    assert_equals(&NO_MORE_DOCS, &disi.next_doc()?);
    Ok(())
}

fn test_invalid_range() -> Result<()> {
    expect_throws(IAE, || range(5, 4).map(|_| ()));
    Ok(())
}

fn test_invalid_range_min() -> Result<()> {
    expect_throws(IAE, || range(-1, 4).map(|_| ()));
    Ok(())
}

fn test_empty_range() -> Result<()> {
    expect_throws(IAE, || range(7, 7).map(|_| ()));
    Ok(())
}

fn test_range_advance() -> Result<()> {
    let mut disi = range(5, 20)?;
    assert_equals(&-1, &disi.doc_id());
    assert_equals(&5, &disi.next_doc()?);
    assert_equals(&17, &disi.advance(17)?);
    assert_equals(&18, &disi.next_doc()?);
    assert_equals(&19, &disi.next_doc()?);
    assert_equals(&NO_MORE_DOCS, &disi.next_doc()?);
    Ok(())
}

/// `new FilterDocIdSetIterator(expectedDisi)`, used for the default
/// `intoBitSet`.
///
/// **Divergence from Lucene 10.5.0.** Java's `FilterDocIdSetIterator` does
/// not override `intoBitSet`, so wrapping selects the default
/// implementation; Rucene's `FilterDocIdSetIterator` delegates it and owns
/// the wrapped iterator. This by-reference filter forwards only `docID`,
/// `nextDoc`, `advance` and `cost`, keeping the trait's default
/// `into_bit_set`, as Java's does, over the iterator the test keeps.
struct DefaultIntoBitSet<'a>(&'a mut dyn DocIdSetIterator);

impl DocIdSetIterator for DefaultIntoBitSet<'_> {
    fn doc_id(&self) -> i32 {
        self.0.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.0.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.advance(target)
    }

    fn cost(&self) -> i64 {
        self.0.cost()
    }
}

fn bits(set: &FixedBitSet) -> Vec<u64> {
    set.get_bits()
        .iter()
        .map(|w| w.load(Ordering::Relaxed))
        .collect()
}

fn test_into_bitset() -> Result<()> {
    for i in 0..10 {
        let max = 1 + random().next_int_bound(500);
        let mut expected_disi: Box<dyn DocIdSetIterator>;
        let mut actual_disi: Box<dyn DocIdSetIterator>;
        if (i & 1) == 0 {
            let min = random().next_int_bound(max);
            expected_disi = Box::new(range(min, max)?);
            actual_disi = Box::new(range(min, max)?);
        } else {
            expected_disi = Box::new(all(max)?);
            actual_disi = Box::new(all(max)?);
        }
        let mut expected = FixedBitSet::new((max * 2) as usize);
        let mut actual = FixedBitSet::new((max * 2) as usize);
        let mut doc = -1;
        expected_disi.next_doc()?;
        actual_disi.next_doc()?;
        while doc != NO_MORE_DOCS {
            let r = random().next_int_bound(3);
            match r {
                0 => {
                    expected_disi.next_doc()?;
                    actual_disi.next_doc()?;
                }
                1 => {
                    let jump = expected_disi.doc_id() + random().next_int_bound(5);
                    expected_disi.advance(jump)?;
                    actual_disi.advance(jump)?;
                }
                2 => {
                    expected.clear_all();
                    actual.clear_all();
                    let up_to = if random().next_boolean() {
                        expected_disi.doc_id() - 1
                    } else {
                        expected_disi.doc_id() + random().next_int_bound(5)
                    };
                    let offset = expected_disi.doc_id() - random().next_int_bound(max);
                    // use the default impl of intoBitSet
                    DefaultIntoBitSet(&mut *expected_disi).into_bit_set(
                        up_to,
                        &mut expected,
                        offset,
                    )?;
                    actual_disi.into_bit_set(up_to, &mut actual, offset)?;
                    assert_array_equals(&bits(&expected), &bits(&actual));
                }
                _ => {}
            }
            assert_equals(&expected_disi.doc_id(), &actual_disi.doc_id());
            doc = expected_disi.doc_id();
        }
    }
    Ok(())
}

fn test_doc_id_run_end_all() -> Result<()> {
    let mut it = all(13)?;
    assert_equals(&0, &it.next_doc()?);
    assert_equals(&13, &it.doc_id_run_end()?);
    assert_equals(&10, &it.advance(10)?);
    assert_equals(&13, &it.doc_id_run_end()?);
    assert_equals(&NO_MORE_DOCS, &it.advance(13)?);
    Ok(())
}

fn test_doc_id_run_end_range() -> Result<()> {
    let mut it = range(4, 13)?;
    assert_equals(&4, &it.next_doc()?);
    assert_equals(&13, &it.doc_id_run_end()?);
    assert_equals(&10, &it.advance(10)?);
    assert_equals(&13, &it.doc_id_run_end()?);
    assert_equals(&NO_MORE_DOCS, &it.advance(13)?);
    Ok(())
}
