//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSkipBlockRangeIterator.java`.
//!
//! `TestSkipBlockRangeIterator extends BaseDocValuesSkipperTests`, whose
//! helpers are `base_doc_values_skipper_tests`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::skip_block_range_iterator::SkipBlockRangeIterator;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::FixedBitSet;

use crate::core_search::base_doc_values_skipper_tests::doc_values_skipper;
use crate::core_support::{assert_equals, class, test};

/// `TestSkipBlockRangeIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSkipBlockRangeIterator",
        vec![
            test("testSkipBlockRangeIterator", test_skip_block_range_iterator),
            test("testIntoBitSet", test_into_bit_set),
        ],
    )
}

/// Port of `TestSkipBlockRangeIterator.testSkipBlockRangeIterator`.
fn test_skip_block_range_iterator() -> Result<()> {
    let skipper = doc_values_skipper(10, 20, true);
    let mut it = SkipBlockRangeIterator::new(skipper, 10, 20);

    assert_equals(&0, &it.next_doc()?);
    // Level-1 block [0,127] is dense and in range: docIDRunEnd expands
    assert_equals(&128, &it.doc_id_run_end()?);
    assert_equals(&100, &it.advance(100)?);
    assert_equals(&512, &it.advance(300)?);
    assert_equals(&513, &it.doc_id_run_end()?);
    // Block [1024,1087] is dense YES, but level-1 [1024,1151] is sparse: can't expand
    assert_equals(&1024, &it.advance(1024)?);
    assert_equals(&1088, &it.doc_id_run_end()?);
    assert_equals(&1100, &it.advance(1100)?);
    assert_equals(&1101, &it.doc_id_run_end()?);
    assert_equals(&1536, &it.advance(1500)?);
    assert_equals(&1537, &it.doc_id_run_end()?);
    assert_equals(&NO_MORE_DOCS, &it.advance(2050)?);
    Ok(())
}

/// Port of `TestSkipBlockRangeIterator.testIntoBitSet`.
fn test_into_bit_set() -> Result<()> {
    let skipper = doc_values_skipper(10, 20, true);
    let mut it = SkipBlockRangeIterator::new(skipper, 10, 20);
    assert_equals(&512, &it.advance(300)?);
    let mut bit_set = FixedBitSet::new(2048);
    it.into_bit_set(1500, &mut bit_set, 512)?;

    let mut expected = FixedBitSet::new(2048);
    expected.set_range(0, 640);

    assert_equals(&expected, &bit_set);
    Ok(())
}
