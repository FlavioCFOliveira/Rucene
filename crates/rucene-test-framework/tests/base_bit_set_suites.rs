//! The `BaseBitSetTestCase` suites (rmp #350): the inherited cases of
//! `BaseBitSetTestCase` run, under the ported runner and `LuceneTestCase`'s
//! rule chain, on the sets Lucene's own subclasses bind
//! (`lucene/core/src/test/org/apache/lucene/util/`):
//!
//! * `TestSparseFixedBitSet` (`BaseBitSetTestCase<SparseFixedBitSet>`), with
//!   its override of `assertEquals(BitSet, SparseFixedBitSet, int)`, which
//!   checks the sparse set's invariants;
//! * `TestFixedBitSet` (`BaseBitSetTestCase<FixedBitSet>`), with its
//!   override of `testPrevSetBit()` (and the helpers that override calls).
//!
//! The classes' own extra test methods belong to the core-test port
//! (sprint 7) and are not declared here.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead. `test_bit_set_suite_draws_match_lucene`
//! compares the seeded draws with `BaseSuitesDrawsOracle`'s
//! (`base_bit_set_draws.tsv`).

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::NO_MORE_DOCS;
use rucene::util::bit_set::BitSet;
use rucene::util::bit_sets::SparseFixedBitSet;
use rucene::util::FixedBitSet;
use rucene_test_framework::jdk::bit_set::JavaBitSet;
use rucene_test_framework::util::base_bit_set_test_case::{
    self, base_assert_equals_sets, BaseBitSetTestCase, BitSetTestInstance,
};
use rucene_test_framework::util::LuceneTestCase;

mod draw_hooks;
mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// `copyOf` of `TestSparseFixedBitSet` and `TestFixedBitSet`: every set bit
/// below `length`, set in order.
fn copy_set_bits(bs: &dyn BitSet, length: usize, mut set: impl FnMut(usize)) {
    let mut doc = bs.next_set_bit(0);
    while doc != NO_MORE_DOCS {
        set(doc as usize);
        doc = if doc as usize + 1 >= length {
            NO_MORE_DOCS
        } else {
            bs.next_set_bit(doc + 1)
        };
    }
}

/// `TestSparseFixedBitSet extends BaseBitSetTestCase<SparseFixedBitSet>`.
struct TestSparseFixedBitSet;

impl BaseBitSetTestCase for TestSparseFixedBitSet {
    type T = SparseFixedBitSet;

    fn copy_of(&self, bs: &dyn BitSet, length: usize) -> Result<SparseFixedBitSet> {
        let mut set = SparseFixedBitSet::new(length)?;
        copy_set_bits(bs, length, |doc| set.set(doc));
        Ok(set)
    }

    fn assert_equals_sets(&self, set1: &dyn BitSet, set2: &SparseFixedBitSet, max_doc: usize) {
        base_assert_equals_sets(set1, set2, max_doc);
        // check invariants of the sparse set
        let mut non_zero_long_count = 0usize;
        for i in 0..set2.indices().len() {
            let n = set2.indices()[i].count_ones() as usize;
            if n != 0 {
                non_zero_long_count += n;
                let block = set2.bits_blocks()[i].as_deref().unwrap_or(&[]);
                for &word in block.iter().skip(n) {
                    Assert::assert_equals(0i64, word as i64);
                }
            }
        }
        Assert::assert_equals(non_zero_long_count, set2.non_zero_long_count());
    }
}

/// `TestFixedBitSet extends BaseBitSetTestCase<FixedBitSet>`.
struct TestFixedBitSet;

impl TestFixedBitSet {
    /// `doPrevSetBit(java.util.BitSet a, FixedBitSet b)`.
    fn do_prev_set_bit(a: &JavaBitSet, b: &FixedBitSet) {
        Assert::assert_equals(a.cardinality(), BitSet::cardinality(b));
        let mut aa = a.size() as i32 + LuceneTestCase::random().next_int_bound(100);
        let mut bb = aa;
        loop {
            // aa = a.prevSetBit(aa-1);
            aa -= 1;
            while aa >= 0 && !a.get(aa as usize) {
                aa -= 1;
            }
            let length = b.length() as i32;
            if length == 0 {
                bb = -1;
            } else if bb > length - 1 {
                bb = b.prev_set_bit(length - 1);
            } else if bb < 1 {
                bb = -1;
            } else {
                bb = if bb >= 1 { b.prev_set_bit(bb - 1) } else { -1 };
            }
            Assert::assert_equals(aa, bb);
            if aa < 0 {
                break;
            }
        }
    }

    /// `makeFixedBitSet(int[] a, int numBits)`.
    fn make_fixed_bit_set(a: &[usize], num_bits: usize) -> Result<FixedBitSet> {
        let mut bs = if LuceneTestCase::random().next_boolean() {
            let bits2words = FixedBitSet::bits2words(num_bits);
            let words =
                vec![0u64; bits2words + LuceneTestCase::random().next_int_bound(100) as usize];
            FixedBitSet::from_bits(words, num_bits)?
        } else {
            FixedBitSet::new(num_bits)
        };
        for &e in a {
            bs.set(e);
        }
        Ok(bs)
    }

    /// `makeBitSet(int[] a)`.
    fn make_bit_set(a: &[usize]) -> JavaBitSet {
        let mut bs = JavaBitSet::new();
        for &e in a {
            bs.set(e);
        }
        bs
    }

    /// `checkPrevSetBitArray(int[] a, int numBits)`.
    fn check_prev_set_bit_array(a: &[usize], num_bits: usize) -> Result<()> {
        let obs = Self::make_fixed_bit_set(a, num_bits)?;
        let bs = Self::make_bit_set(a);
        Self::do_prev_set_bit(&bs, &obs);
        Ok(())
    }
}

impl BaseBitSetTestCase for TestFixedBitSet {
    type T = FixedBitSet;

    fn copy_of(&self, bs: &dyn BitSet, length: usize) -> Result<FixedBitSet> {
        let mut set = FixedBitSet::new(length);
        copy_set_bits(bs, length, |doc| set.set(doc));
        Ok(set)
    }

    /// `TestFixedBitSet.testPrevSetBit()` (`@Override`).
    fn test_prev_set_bit(&self) -> Result<()> {
        Self::check_prev_set_bit_array(&[], 0)?;
        Self::check_prev_set_bit_array(&[0], 1)?;
        Self::check_prev_set_bit_array(&[0, 2], 3)
    }
}

fn sparse_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.util.TestSparseFixedBitSet")
        .extends(&base_bit_set_test_case::class())
        .constructor(Constructor::no_args(|| {
            BitSetTestInstance::new(TestSparseFixedBitSet)
        }))
        .build()
}

fn fixed_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.util.TestFixedBitSet")
        .extends(&base_bit_set_test_case::class())
        .constructor(Constructor::no_args(|| {
            BitSetTestInstance::new(TestFixedBitSet)
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestSparseFixedBitSetDraws, TestFixedBitSetDraws
// -----------------------------------------------------------------------------

static SPARSE_DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FIXED_DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestSparseFixedBitSetDraws` of `BaseSuitesDrawsOracle`.
fn sparse_draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.util.TestSparseFixedBitSetDraws")
        .extends(&base_bit_set_test_case::class())
        .constructor(Constructor::no_args(|| {
            BitSetTestInstance::new(TestSparseFixedBitSet)
        }))
        .method(draw_hooks::record_tear_down(
            &SPARSE_DRAWS,
            draw_hooks::lucene_tear_down::<BitSetTestInstance>,
        ))
        .method(draw_hooks::record_class_draw(&SPARSE_DRAWS))
        .build()
}

/// `TestFixedBitSetDraws` of `BaseSuitesDrawsOracle`.
fn fixed_draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.util.TestFixedBitSetDraws")
        .extends(&base_bit_set_test_case::class())
        .constructor(Constructor::no_args(|| {
            BitSetTestInstance::new(TestFixedBitSet)
        }))
        .method(draw_hooks::record_tear_down(
            &FIXED_DRAWS,
            draw_hooks::lucene_tear_down::<BitSetTestInstance>,
        ))
        .method(draw_hooks::record_class_draw(&FIXED_DRAWS))
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_sparse_fixed_bit_set_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match(
        "base_sparse_fixed_bit_set_draws.tsv",
        sparse_draws_class,
        &SPARSE_DRAWS,
    );
}

#[test]
fn test_fixed_bit_set_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match(
        "base_fixed_bit_set_draws.tsv",
        fixed_draws_class,
        &FIXED_DRAWS,
    );
}

#[test]
fn test_sparse_fixed_bit_set() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(sparse_class);
}

#[test]
fn test_fixed_bit_set() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(fixed_class);
}
