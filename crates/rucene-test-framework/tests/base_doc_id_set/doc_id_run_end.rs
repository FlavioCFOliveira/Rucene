//! The private static helpers `TestRoaringDocIdSet` and `TestNotDocIdSet`
//! both declare: `randomBitSet`, `expectedDocIDRunEnd` and
//! `assertDocIDRunEndMatches`.

use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::NO_MORE_DOCS;
use rucene_test_framework::jdk::bit_set::JavaBitSet;
use rucene_test_framework::util::LuceneTestCase;

/// `randomBitSet(int numBits, float percentSet)`.
pub fn random_bit_set(num_bits: i32, percent_set: f32) -> JavaBitSet {
    let num_bits_set = num_bits.min((percent_set * num_bits as f32) as i32);
    let mut set = JavaBitSet::with_capacity(num_bits as usize);
    if num_bits_set >= num_bits {
        set.set_range(0, num_bits as usize);
        return set;
    }
    for _ in 0..num_bits_set {
        loop {
            let o = LuceneTestCase::random().next_int_bound(num_bits);
            if !set.get(o as usize) {
                set.set(o as usize);
                break;
            }
        }
    }
    set
}

/// `expectedDocIDRunEnd(BitSet bs, int maxDoc, int doc)`.
pub fn expected_doc_id_run_end(bs: &JavaBitSet, max_doc: i32, doc: i32) -> i32 {
    let mut end = doc + 1;
    while end < max_doc && bs.get(end as usize) {
        end += 1;
    }
    end
}

/// `assertDocIDRunEndMatches(BitSet bs, int maxDoc, T set)`.
pub fn assert_doc_id_run_end_matches(
    bs: &JavaBitSet,
    max_doc: i32,
    set: &dyn DocIdSet,
) -> Result<()> {
    // assertNotNull(it): Rucene's `iterator()` never returns null.
    let mut it = set.iterator()?;
    let mut doc = it.next_doc()?;
    while doc != NO_MORE_DOCS {
        Assert::assert_true(bs.get(doc as usize));
        Assert::assert_equals(
            expected_doc_id_run_end(bs, max_doc, doc),
            it.doc_id_run_end()?,
        );
        doc = it.next_doc()?;
    }
    Ok(())
}
