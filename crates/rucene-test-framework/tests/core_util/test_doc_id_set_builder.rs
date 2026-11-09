//! Port of `lucene/core/src/test/org/apache/lucene/util/TestDocIdSetBuilder.java`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::point_values::{PointTree, PointValues};
use rucene::index::{Terms, TermsEnum};
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::doc_id_set_iterator::range;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::bit_set::{BitSet, BitSetIterator};
use rucene::util::doc_id_set::{BitDocIdSet, DocIdSetBuilder, IntArrayDocIdSet};
use rucene::util::{FixedBitSet, IntsRef, RoaringDocIdSet};
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestDocIdSetBuilder`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestDocIdSetBuilder",
        vec![
            test("testEmpty", test_empty),
            test("testSparse", test_sparse),
            test("testDense", test_dense),
            test("testRandom", test_random),
            test("testMisleadingDISICost", test_misleading_disi_cost),
            test("testEmptyPoints", test_empty_points),
            test("testLeverageStats", test_leverage_stats),
            test(
                "testCostIsCorrectAfterBitsetUpgrade",
                test_cost_is_correct_after_bitset_upgrade,
            ),
        ],
    )
}

/// `d instanceof T`.
fn is_instance<T: 'static>(d: &dyn DocIdSet) -> bool {
    d.doc_id_set_as_any().is_some_and(|a| a.is::<T>())
}

fn test_empty() -> Result<()> {
    let set = DocIdSetBuilder::new(1 + random().next_int_bound(1000)).build()?;
    assert_doc_id_sets_equal(None, Some(&*set))
}

/// `assertEquals(DocIdSet d1, DocIdSet d2)`.
fn assert_doc_id_sets_equal(d1: Option<&dyn DocIdSet>, d2: Option<&dyn DocIdSet>) -> Result<()> {
    match (d1, d2) {
        (None, d2) => {
            if let Some(d2) = d2 {
                assert_equals(&NO_MORE_DOCS, &d2.iterator()?.next_doc()?);
            }
        }
        (Some(d1), None) => {
            assert_equals(&NO_MORE_DOCS, &d1.iterator()?.next_doc()?);
        }
        (Some(d1), Some(d2)) => {
            let mut i1 = d1.iterator()?;
            let mut i2 = d2.iterator()?;
            let mut doc = i1.next_doc()?;
            while doc != NO_MORE_DOCS {
                assert_equals(&doc, &i2.next_doc()?);
                doc = i1.next_doc()?;
            }
            assert_equals(&NO_MORE_DOCS, &i2.next_doc()?);
        }
    }
    Ok(())
}

/// `new BitDocIdSet(ref)`.
fn bit_doc_id_set(set: FixedBitSet) -> Result<BitDocIdSet> {
    BitDocIdSet::from_set(Arc::new(set))
}

fn test_sparse() -> Result<()> {
    let max_doc = 1000000 + random().next_int_bound(1000000);
    let mut builder = DocIdSetBuilder::new(max_doc);
    let num_iterators = 1 + random().next_int_bound(10);
    let mut r#ref = FixedBitSet::new(max_doc as usize);
    for _ in 0..num_iterators {
        let base_inc = 200000 + random().next_int_bound(10000);
        let mut b = RoaringDocIdSet::builder(max_doc as usize);
        let mut doc = random().next_int_bound(100);
        while doc < max_doc {
            b.add(doc as usize)?;
            r#ref.set(doc as usize);
            doc += base_inc + random().next_int_bound(10000);
        }
        builder.add(&mut *b.build().iterator()?)?;
    }
    let result = builder.build()?;
    Assert::assert_true(is_instance::<IntArrayDocIdSet>(&*result));
    assert_doc_id_sets_equal(Some(&bit_doc_id_set(r#ref)?), Some(&*result))
}

fn test_dense() -> Result<()> {
    let max_doc = 1000000 + random().next_int_bound(1000000);
    let mut builder = DocIdSetBuilder::new(max_doc);
    let num_iterators = 1 + random().next_int_bound(10);
    let mut r#ref = FixedBitSet::new(max_doc as usize);
    for _ in 0..num_iterators {
        let mut b = RoaringDocIdSet::builder(max_doc as usize);
        let mut doc = random().next_int_bound(1000);
        while doc < max_doc {
            b.add(doc as usize)?;
            r#ref.set(doc as usize);
            doc += 1 + random().next_int_bound(100);
        }
        builder.add(&mut *b.build().iterator()?)?;
    }
    let result = builder.build()?;
    Assert::assert_true(is_instance::<BitDocIdSet>(&*result));
    assert_doc_id_sets_equal(Some(&bit_doc_id_set(r#ref)?), Some(&*result))
}

fn test_random() -> Result<()> {
    let max_doc = if *TEST_NIGHTLY {
        TestUtil::next_int(&*random(), 1, 10000000)
    } else {
        TestUtil::next_int(&*random(), 1, 100000)
    };
    let mut i = 1;
    while i < max_doc / 2 {
        let num_docs = TestUtil::next_int(&*random(), 1, i);
        let mut docs = FixedBitSet::new(max_doc as usize);
        let mut c = 0;
        while c < num_docs {
            let d = random().next_int_bound(max_doc) as usize;
            if !docs.get(d) {
                docs.set(d);
                c += 1;
            }
        }
        let docs: Arc<dyn BitSet> = Arc::new(docs);

        let mut array: Vec<i32> = vec![0; (num_docs + random().next_int_bound(100)) as usize];
        let mut it = BitSetIterator::new(Arc::clone(&docs), 0)?;
        let mut j = 0usize;
        let mut doc = it.next_doc()?;
        while doc != NO_MORE_DOCS {
            array[j] = doc;
            j += 1;
            doc = it.next_doc()?;
        }
        assert_equals(&(num_docs as usize), &j);

        // add some duplicates
        while j < array.len() {
            array[j] = array[random().next_int_bound(num_docs) as usize];
            j += 1;
        }

        // shuffle
        j = array.len() - 1;
        while j >= 1 {
            let k = random().next_int_bound(j as i32) as usize;
            array.swap(j, k);
            j -= 1;
        }

        // add docs out of order
        let mut builder = DocIdSetBuilder::new(max_doc);
        j = 0;
        while j < array.len() {
            let l = TestUtil::next_int(&*random(), 1, (array.len() - j) as i32);
            if LuceneTestCase::usually() {
                let mut adder = None;
                let mut budget = 0;
                for k in 0..l {
                    if budget == 0 || LuceneTestCase::rarely() {
                        budget = TestUtil::next_int(&*random(), 1, l - k + 5);
                        drop(adder.take());
                        adder = Some(builder.grow(budget));
                    }
                    adder.as_mut().expect("adder").add(array[j]);
                    j += 1;
                    budget -= 1;
                }
            } else {
                let ints_ref = IntsRef::with_offset_length(array.clone(), j, l as usize);
                let mut adder = builder.grow(l);
                adder.add_ints(&ints_ref);
                j += l as usize;
            }
        }

        let expected = BitDocIdSet::from_set(Arc::clone(&docs))?;
        let actual = builder.build()?;
        assert_doc_id_sets_equal(Some(&expected), Some(&*actual))?;
        i <<= 1;
    }
    Ok(())
}

fn test_misleading_disi_cost() -> Result<()> {
    let max_doc = TestUtil::next_int(&*random(), 1000, 10000);
    let mut builder = DocIdSetBuilder::new(max_doc);
    let mut expected = FixedBitSet::new(max_doc as usize);

    for _ in 0..10 {
        let mut docs = FixedBitSet::new(max_doc as usize);
        let num_docs = random().next_int_bound(max_doc / 1000);
        for _ in 0..num_docs {
            docs.set(random().next_int_bound(max_doc) as usize);
        }
        expected.or_set(&docs);
        // We provide a cost of 0 here to make sure the builder can deal with wrong costs
        builder.add(&mut BitSetIterator::new(Arc::new(docs), 0)?)?;
    }

    let built = builder.build()?;
    assert_doc_id_sets_equal(Some(&bit_doc_id_set(expected)?), Some(&*built))
}

fn test_empty_points() -> Result<()> {
    let values = DummyPointValues::new(0, 0);
    let builder = DocIdSetBuilder::from_point_values(1, &values)?;
    assert_equals_delta(1.0, builder.num_values_per_doc(), 0.0);
    Ok(())
}

fn test_leverage_stats() -> Result<()> {
    // single-valued points
    let mut values = DummyPointValues::new(42, 42);
    let mut builder = DocIdSetBuilder::from_point_values(100, &values)?;
    assert_equals_delta(1.0, builder.num_values_per_doc(), 0.0);
    Assert::assert_false(builder.multivalued());
    {
        let mut adder = builder.grow(2);
        adder.add(5);
        adder.add(7);
    }
    let mut set = builder.build()?;
    Assert::assert_true(is_instance::<BitDocIdSet>(&*set));
    assert_equals(&2, &set.iterator()?.cost());

    // multi-valued points
    values = DummyPointValues::new(42, 63);
    builder = DocIdSetBuilder::from_point_values(100, &values)?;
    assert_equals_delta(1.5, builder.num_values_per_doc(), 0.0);
    Assert::assert_true(builder.multivalued());
    {
        let mut adder = builder.grow(2);
        adder.add(5);
        adder.add(7);
    }
    set = builder.build()?;
    Assert::assert_true(is_instance::<BitDocIdSet>(&*set));
    assert_equals(&1, &set.iterator()?.cost()); // it thinks the same doc was added twice

    // incomplete stats
    values = DummyPointValues::new(42, -1);
    builder = DocIdSetBuilder::from_point_values(100, &values)?;
    assert_equals_delta(1.0, builder.num_values_per_doc(), 0.0);
    Assert::assert_true(builder.multivalued());

    values = DummyPointValues::new(-1, 84);
    builder = DocIdSetBuilder::from_point_values(100, &values)?;
    assert_equals_delta(1.0, builder.num_values_per_doc(), 0.0);
    Assert::assert_true(builder.multivalued());

    // single-valued terms
    let mut terms = DummyTerms::new(42, 42);
    builder = DocIdSetBuilder::from_terms(100, &terms)?;
    assert_equals_delta(1.0, builder.num_values_per_doc(), 0.0);
    Assert::assert_false(builder.multivalued());
    {
        let mut adder = builder.grow(2);
        adder.add(5);
        adder.add(7);
    }
    set = builder.build()?;
    Assert::assert_true(is_instance::<BitDocIdSet>(&*set));
    assert_equals(&2, &set.iterator()?.cost());

    // multi-valued terms
    terms = DummyTerms::new(42, 63);
    builder = DocIdSetBuilder::from_terms(100, &terms)?;
    assert_equals_delta(1.5, builder.num_values_per_doc(), 0.0);
    Assert::assert_true(builder.multivalued());
    {
        let mut adder = builder.grow(2);
        adder.add(5);
        adder.add(7);
    }
    set = builder.build()?;
    Assert::assert_true(is_instance::<BitDocIdSet>(&*set));
    assert_equals(&1, &set.iterator()?.cost()); // it thinks the same doc was added twice

    // incomplete stats
    terms = DummyTerms::new(42, -1);
    builder = DocIdSetBuilder::from_terms(100, &terms)?;
    assert_equals_delta(1.0, builder.num_values_per_doc(), 0.0);
    Assert::assert_true(builder.multivalued());

    terms = DummyTerms::new(-1, 84);
    builder = DocIdSetBuilder::from_terms(100, &terms)?;
    assert_equals_delta(1.0, builder.num_values_per_doc(), 0.0);
    Assert::assert_true(builder.multivalued());
    Ok(())
}

fn test_cost_is_correct_after_bitset_upgrade() -> Result<()> {
    let max_doc = 1000000;
    let mut builder = DocIdSetBuilder::new(max_doc);
    // 1000000 >> 6 is greater than DocIdSetBuilder.threshold which is 1000000 >> 7
    for i in 0..(1000000 >> 6) {
        builder.add(&mut range(i, i + 1)?)?;
    }
    let result = builder.build()?;
    Assert::assert_true(is_instance::<BitDocIdSet>(&*result));
    assert_equals(&(1000000i64 >> 6), &result.iterator()?.cost());
    Ok(())
}

/// `throw new UnsupportedOperationException()`.
fn unsupported() -> ! {
    Throwable::new("java.lang.UnsupportedOperationException", "").throw()
}

/// `DummyTerms extends Terms`.
struct DummyTerms {
    doc_count: i32,
    num_values: i64,
}

impl DummyTerms {
    fn new(doc_count: i32, num_values: i64) -> DummyTerms {
        DummyTerms {
            doc_count,
            num_values,
        }
    }
}

impl Terms for DummyTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        unsupported()
    }

    fn size(&self) -> Result<i64> {
        unsupported()
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        unsupported()
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.num_values)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.doc_count)
    }

    fn has_freqs(&self) -> bool {
        unsupported()
    }

    fn has_offsets(&self) -> bool {
        unsupported()
    }

    fn has_positions(&self) -> bool {
        unsupported()
    }

    fn has_payloads(&self) -> bool {
        unsupported()
    }
}

/// `DummyPointValues extends PointValues`.
struct DummyPointValues {
    doc_count: i32,
    num_points: i64,
}

impl DummyPointValues {
    fn new(doc_count: i32, num_points: i64) -> DummyPointValues {
        DummyPointValues {
            doc_count,
            num_points,
        }
    }
}

impl PointValues for DummyPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        unsupported()
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        unsupported()
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        unsupported()
    }

    fn num_dimensions(&self) -> Result<i32> {
        unsupported()
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        unsupported()
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        unsupported()
    }

    fn size(&self) -> Result<i64> {
        Ok(self.num_points)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.doc_count)
    }
}
