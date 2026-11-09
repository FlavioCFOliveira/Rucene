//! The `BaseDocIdSetTestCase` suites (rmp #350): the inherited cases of
//! `BaseDocIdSetTestCase` run, under the ported runner and `LuceneTestCase`'s
//! rule chain, on the sets Lucene's own subclasses bind
//! (`lucene/core/src/test/org/apache/lucene/util/`): `TestRoaringDocIdSet`,
//! `TestIntArrayDocIdSet`, `TestNotDocIdSet`, `TestSparseFixedBitDocIdSet`
//! and `TestFixedBitDocIdSet`, each with its `copyOf` and its override of
//! `assertEquals(int, BitSet, T)`.
//!
//! The classes' own extra test methods belong to the core-test port
//! (sprint 7) and are not declared here.
//!
//! Each set is wrapped in a [`JavaShaped`], which describes the Java layout
//! `RamUsageTester` measures in `testRamBytesUsed` (see
//! `base_doc_id_set_test_case`'s declared divergence).
//!
//! `RUCENE_SUITE_SEEDS=<n>` runs each suite on the first `n` seeds of
//! `suite_seeds::SEEDS`; `test_*_suite_draws_match_lucene` compare the
//! seeded draws with `BaseSuitesDrawsOracle`'s.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::NO_MORE_DOCS;
use rucene::util::bit_sets::{RoaringBlockLayout, RoaringDocIdSet, SparseFixedBitSet};
use rucene::util::doc_id_set::{BitDocIdSet, IntArrayDocIdSet, NotDocIdSet};
use rucene::util::{ArrayUtil, FixedBitSet, RamUsageEstimator};
use rucene_test_framework::jdk::bit_set::JavaBitSet;
use rucene_test_framework::util::base_doc_id_set_test_case::{
    self, base_assert_equals_sets, BaseDocIdSetTestCase, DocIdSetTestInstance, JavaShape,
    JavaShaped,
};
use rucene_test_framework::util::LuceneTestCase;

mod draw_hooks;
mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// The Java layout of a `SparseFixedBitSet`: `long[] indices`, `long[][]
/// bits`, `int length`, `int nonZeroLongCount`, `long ramBytesUsed`.
fn sparse_shape(set: &SparseFixedBitSet) -> Arc<JavaShape> {
    Arc::new(JavaShape::Object {
        class: "org.apache.lucene.util.SparseFixedBitSet",
        field_sizes: vec![
            RamUsageEstimator::NUM_BYTES_OBJECT_REF,
            RamUsageEstimator::NUM_BYTES_OBJECT_REF,
            4,
            4,
            8,
        ],
        fields: vec![
            ("indices", Some(JavaShape::longs(set.indices().len()))),
            (
                "bits",
                Some(Arc::new(JavaShape::ObjectArray {
                    class: "[[J",
                    elements: set
                        .bits_blocks()
                        .iter()
                        .map(|b| b.as_ref().map(|b| JavaShape::longs(b.len())))
                        .collect(),
                })),
            ),
        ],
    })
}

/// `RoaringDocIdSet.AllDocIdSet.INSTANCE`, the singleton every full block
/// shares.
static ALL_DOC_ID_SET: LazyLock<Arc<JavaShape>> = LazyLock::new(|| {
    Arc::new(JavaShape::Object {
        class: "org.apache.lucene.util.RoaringDocIdSet$AllDocIdSet",
        field_sizes: vec![],
        fields: vec![],
    })
});

/// A `RoaringDocIdSet.ShortArrayDocIdSet` of `len` docs: `short[] docIDs`.
fn short_array_shape(len: usize) -> Arc<JavaShape> {
    Arc::new(JavaShape::Object {
        class: "org.apache.lucene.util.RoaringDocIdSet$ShortArrayDocIdSet",
        field_sizes: vec![RamUsageEstimator::NUM_BYTES_OBJECT_REF],
        fields: vec![("docIDs", Some(JavaShape::shorts(len)))],
    })
}

/// The Java layout of a `RoaringDocIdSet`: `DocIdSet[] docIdSets`, `int
/// cardinality`, `long ramBytesUsed`.
fn roaring_shape(set: &RoaringDocIdSet) -> Arc<JavaShape> {
    let elements = set
        .block_layouts()
        .into_iter()
        .map(|b| {
            b.map(|b| match b {
                RoaringBlockLayout::All => Arc::clone(&ALL_DOC_ID_SET),
                RoaringBlockLayout::Range => Arc::new(JavaShape::Object {
                    class: "org.apache.lucene.util.RoaringDocIdSet$RangeDocIdSet",
                    field_sizes: vec![2, 2],
                    fields: vec![],
                }),
                RoaringBlockLayout::ShortArray(len) => short_array_shape(len),
                RoaringBlockLayout::BitSet(words) => {
                    JavaShape::bit_doc_id_set(JavaShape::fixed_bit_set(words))
                }
                RoaringBlockLayout::NotShortArray(len) => {
                    JavaShape::not_doc_id_set(short_array_shape(len))
                }
            })
        })
        .collect();
    Arc::new(JavaShape::Object {
        class: "org.apache.lucene.util.RoaringDocIdSet",
        field_sizes: vec![RamUsageEstimator::NUM_BYTES_OBJECT_REF, 4, 8],
        fields: vec![(
            "docIdSets",
            Some(Arc::new(JavaShape::ObjectArray {
                class: "[Lorg.apache.lucene.search.DocIdSet;",
                elements,
            })),
        )],
    })
}

/// `TestRoaringDocIdSet extends BaseDocIdSetTestCase<RoaringDocIdSet>`.
struct TestRoaringDocIdSet;

impl BaseDocIdSetTestCase for TestRoaringDocIdSet {
    type T = JavaShaped<RoaringDocIdSet>;

    fn copy_of(&self, bs: &JavaBitSet, length: i32) -> Result<Self::T> {
        let mut builder = RoaringDocIdSet::builder(length as usize);
        let mut i = bs.next_set_bit(0);
        while i != -1 {
            builder.add(i as usize)?;
            i = bs.next_set_bit(i as usize + 1);
        }
        let set = builder.build();
        let shape = roaring_shape(&set);
        Ok(JavaShaped { set, shape })
    }

    fn assert_equals_sets(&self, num_bits: i32, ds1: &JavaBitSet, ds2: &Self::T) -> Result<()> {
        base_assert_equals_sets(num_bits, ds1, ds2)?;
        Assert::assert_equals(ds1.cardinality(), ds2.cardinality());
        Ok(())
    }
}

/// `TestIntArrayDocIdSet extends BaseDocIdSetTestCase<IntArrayDocIdSet>`.
struct TestIntArrayDocIdSet;

impl BaseDocIdSetTestCase for TestIntArrayDocIdSet {
    type T = JavaShaped<IntArrayDocIdSet>;

    fn copy_of(&self, bs: &JavaBitSet, length: i32) -> Result<Self::T> {
        let mut docs: Vec<i32> = Vec::new();
        let mut l = 0usize;
        let mut i = bs.next_set_bit(0);
        while i != -1 {
            docs = ArrayUtil::grow_int(&docs, length as usize + 1);
            docs[l] = i;
            l += 1;
            i = bs.next_set_bit(i as usize + 1);
        }
        docs = ArrayUtil::grow_int(&docs, length as usize + 1);
        docs[l] = NO_MORE_DOCS;
        let shape = Arc::new(JavaShape::Object {
            class: "org.apache.lucene.util.IntArrayDocIdSet",
            field_sizes: vec![RamUsageEstimator::NUM_BYTES_OBJECT_REF, 4],
            fields: vec![("docs", Some(JavaShape::ints(docs.len())))],
        });
        Ok(JavaShaped {
            set: IntArrayDocIdSet::new(docs, l)?,
            shape,
        })
    }
}

/// `TestNotDocIdSet extends BaseDocIdSetTestCase<NotDocIdSet>`.
struct TestNotDocIdSet;

impl BaseDocIdSetTestCase for TestNotDocIdSet {
    type T = JavaShaped<NotDocIdSet>;

    fn copy_of(&self, bs: &JavaBitSet, length: i32) -> Result<Self::T> {
        let mut set = FixedBitSet::new(length as usize);
        let mut doc = bs.next_clear_bit(0);
        while doc < length {
            set.set(doc as usize);
            doc = bs.next_clear_bit(doc as usize + 1);
        }
        let shape = JavaShape::not_doc_id_set(JavaShape::bit_doc_id_set(JavaShape::fixed_bit_set(
            set.get_bits().len(),
        )));
        Ok(JavaShaped {
            set: NotDocIdSet::new(length, Arc::new(BitDocIdSet::from_set(Arc::new(set))?)),
            shape,
        })
    }

    fn assert_equals_sets(&self, num_bits: i32, ds1: &JavaBitSet, ds2: &Self::T) -> Result<()> {
        base_assert_equals_sets(num_bits, ds1, ds2)?;
        let bits2 = ds2.bits()?;
        // since we wrapped a FixedBitSet
        let bits2 = bits2.unwrap_or_else(|| Assert::fail_without_message());
        Assert::assert_equals(num_bits as usize, bits2.length());
        for i in 0..num_bits as usize {
            Assert::assert_equals(ds1.get(i), bits2.get(i));
        }
        Ok(())
    }
}

/// `TestSparseFixedBitDocIdSet extends BaseDocIdSetTestCase<BitDocIdSet>`.
struct TestSparseFixedBitDocIdSet;

impl BaseDocIdSetTestCase for TestSparseFixedBitDocIdSet {
    type T = JavaShaped<BitDocIdSet>;

    fn copy_of(&self, bs: &JavaBitSet, length: i32) -> Result<Self::T> {
        let mut set = SparseFixedBitSet::new(length as usize)?;
        // SparseFixedBitSet can be sensitive to the order of insertion so
        // randomize insertion a bit
        let mut buffer: Vec<i32> = Vec::new();
        let mut doc = bs.next_set_bit(0);
        while doc != -1 {
            buffer.push(doc);
            if buffer.len() >= 100000 {
                shuffle(&mut buffer, &*LuceneTestCase::random());
                for &i in &buffer {
                    set.set(i as usize);
                }
                buffer.clear();
            }
            doc = bs.next_set_bit(doc as usize + 1);
        }
        shuffle(&mut buffer, &*LuceneTestCase::random());
        for &i in &buffer {
            set.set(i as usize);
        }
        let shape = JavaShape::bit_doc_id_set(sparse_shape(&set));
        let cost = set.approximate_cardinality() as i64;
        Ok(JavaShaped {
            set: BitDocIdSet::new(Arc::new(set), cost)?,
            shape,
        })
    }

    fn assert_equals_sets(&self, num_bits: i32, ds1: &JavaBitSet, ds2: &Self::T) -> Result<()> {
        for i in 0..num_bits as usize {
            Assert::assert_equals(ds1.get(i), ds2.set().get(i));
        }
        Assert::assert_equals(ds1.cardinality(), ds2.set().cardinality());
        base_assert_equals_sets(num_bits, ds1, ds2)
    }
}

/// `TestFixedBitDocIdSet extends BaseDocIdSetTestCase<BitDocIdSet>`.
struct TestFixedBitDocIdSet;

impl BaseDocIdSetTestCase for TestFixedBitDocIdSet {
    type T = JavaShaped<BitDocIdSet>;

    fn copy_of(&self, bs: &JavaBitSet, length: i32) -> Result<Self::T> {
        let mut set = FixedBitSet::new(length as usize);
        let mut doc = bs.next_set_bit(0);
        while doc != -1 {
            set.set(doc as usize);
            doc = bs.next_set_bit(doc as usize + 1);
        }
        let shape = JavaShape::bit_doc_id_set(JavaShape::fixed_bit_set(set.get_bits().len()));
        Ok(JavaShaped {
            set: BitDocIdSet::from_set(Arc::new(set))?,
            shape,
        })
    }
}

/// A binding class `name` of `BaseDocIdSetTestCase`, its instances made by
/// `make`.
fn binding(name: &str, make: fn() -> DocIdSetTestInstance) -> Arc<Class> {
    Class::builder(name)
        .extends(&base_doc_id_set_test_case::class())
        .constructor(Constructor::no_args(make))
        .build()
}

fn roaring_class() -> Arc<Class> {
    binding("org.apache.lucene.util.TestRoaringDocIdSet", || {
        DocIdSetTestInstance::new(TestRoaringDocIdSet)
    })
}

fn int_array_class() -> Arc<Class> {
    binding("org.apache.lucene.util.TestIntArrayDocIdSet", || {
        DocIdSetTestInstance::new(TestIntArrayDocIdSet)
    })
}

fn not_class() -> Arc<Class> {
    binding("org.apache.lucene.util.TestNotDocIdSet", || {
        DocIdSetTestInstance::new(TestNotDocIdSet)
    })
}

fn sparse_class() -> Arc<Class> {
    binding("org.apache.lucene.util.TestSparseFixedBitDocIdSet", || {
        DocIdSetTestInstance::new(TestSparseFixedBitDocIdSet)
    })
}

fn fixed_class() -> Arc<Class> {
    binding("org.apache.lucene.util.TestFixedBitDocIdSet", || {
        DocIdSetTestInstance::new(TestFixedBitDocIdSet)
    })
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: Test…DocIdSetDraws
// -----------------------------------------------------------------------------

static ROARING_DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static SPARSE_DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A draws class of `BaseSuitesDrawsOracle` over a binding's instances.
fn draws_binding(
    name: &str,
    make: fn() -> DocIdSetTestInstance,
    draws: &'static Mutex<Vec<String>>,
) -> Arc<Class> {
    Class::builder(name)
        .extends(&base_doc_id_set_test_case::class())
        .constructor(Constructor::no_args(make))
        .method(draw_hooks::record_tear_down(
            draws,
            draw_hooks::lucene_tear_down::<DocIdSetTestInstance>,
        ))
        .method(draw_hooks::record_class_draw(draws))
        .build()
}

/// `TestRoaringDocIdSetDraws`: the cases on `TestRoaringDocIdSet`'s sets.
fn roaring_draws_class() -> Arc<Class> {
    draws_binding(
        "org.apache.lucene.util.TestRoaringDocIdSetDraws",
        || DocIdSetTestInstance::new(TestRoaringDocIdSet),
        &ROARING_DRAWS,
    )
}

/// `TestSparseFixedBitDocIdSetDraws`: the cases on
/// `TestSparseFixedBitDocIdSet`'s sets (whose `copyOf` draws the shuffles).
fn sparse_draws_class() -> Arc<Class> {
    draws_binding(
        "org.apache.lucene.util.TestSparseFixedBitDocIdSetDraws",
        || DocIdSetTestInstance::new(TestSparseFixedBitDocIdSet),
        &SPARSE_DRAWS,
    )
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_roaring_doc_id_set_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match(
        "base_roaring_doc_id_set_draws.tsv",
        roaring_draws_class,
        &ROARING_DRAWS,
    );
}

#[test]
fn test_sparse_fixed_bit_doc_id_set_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match(
        "base_sparse_fixed_bit_doc_id_set_draws.tsv",
        sparse_draws_class,
        &SPARSE_DRAWS,
    );
}

#[test]
fn test_roaring_doc_id_set() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(roaring_class);
}

#[test]
fn test_int_array_doc_id_set() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(int_array_class);
}

#[test]
fn test_not_doc_id_set() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(not_class);
}

#[test]
fn test_sparse_fixed_bit_doc_id_set() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(sparse_class);
}

#[test]
fn test_fixed_bit_doc_id_set() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(fixed_class);
}
