//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBitSetDocIdStream.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_stream::DocIdStream;
use rucene::search::{
    BitSetDocIdStream, CollectionError, CollectionResult, DocIdSetIterator, NO_MORE_DOCS,
};
use rucene::util::bit_set::{BitSet, BitSetIterator};
use rucene::util::FixedBitSet;
use rucene_test_framework::util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};

/// `TestBitSetDocIdStream`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBitSetDocIdStream",
        vec![
            test("testForEach", test_for_each),
            test("testCount", test_count),
            test("testIntoArray", test_into_array),
            test("testForEachUpTo", test_for_each_up_to),
            test("testCountUpTo", test_count_up_to),
            test("testIntoArrayUpTo", test_into_array_up_to),
            test("testMixForEachCountUpTo", test_mix_for_each_count_up_to),
        ],
    )
}

fn random_bit_set() -> FixedBitSet {
    let mut set = FixedBitSet::new(100);
    let mut i = TestUtil::next_int(&*random(), 0, 5);
    while (i as usize) < set.length() {
        set.set(i as usize);
        i += TestUtil::next_int(&*random(), 1, 5);
    }
    set
}

/// A `CollectionResult` thrown as Java does.
fn coll(r: CollectionResult<()>) -> Result<()> {
    r.map_err(CollectionError::into_lucene_error)
}

/// `new BitSetIterator(bitSet, cost)` over a copy of `bit_set` (Java
/// shares the instance; neither side mutates it).
fn bit_set_iterator(bit_set: &FixedBitSet, cost: i64) -> Result<BitSetIterator> {
    BitSetIterator::new(Arc::new(bit_set.clone()), cost)
}

/// `doc -> assertEquals(base + iterator.nextDoc(), doc)`.
fn expect_next(
    iterator: &mut BitSetIterator,
    base: i32,
) -> impl FnMut(i32) -> CollectionResult<()> + '_ {
    move |doc| {
        assert_equals(&(base + iterator.next_doc()?), &doc);
        Ok(())
    }
}

/// `doc -> fail()`.
fn fail_on_doc(_doc: i32) -> CollectionResult<()> {
    Assert::fail("")
}

fn test_for_each() -> Result<()> {
    let bit_set = random_bit_set();
    let base = 42;
    let mut stream = BitSetDocIdStream::new(&bit_set, base);
    let mut iterator = bit_set_iterator(&bit_set, bit_set.approximate_cardinality() as i64)?;
    coll(stream.for_each(&mut expect_next(&mut iterator, base)))?;
    assert_equals(&NO_MORE_DOCS, &iterator.next_doc()?);
    Ok(())
}

fn test_count() -> Result<()> {
    let bit_set = random_bit_set();
    let base = 42;
    let mut stream = BitSetDocIdStream::new(&bit_set, base);
    assert_equals(&(bit_set.cardinality() as i32), &stream.count()?);
    Ok(())
}

fn test_into_array() -> Result<()> {
    let bit_set = random_bit_set();
    let mut bit_set_iterator = bit_set_iterator(&bit_set, 0)?;
    let base = 42;
    let mut stream = BitSetDocIdStream::new(&bit_set, base);
    let mut array = [0i32; 16];
    let mut o = array.len();
    let mut count = array.len();
    let mut expected = bit_set_iterator.next_doc()?;
    while expected != NO_MORE_DOCS {
        if o == count {
            count = stream.into_array(&mut array);
            o = 0;
        }
        assert_equals(&(expected + base), &array[o]);
        o += 1;
        expected = bit_set_iterator.next_doc()?;
    }
    assert_equals(&count, &o);
    assert_equals(&0, &stream.into_array(&mut array));
    Ok(())
}

fn test_for_each_up_to() -> Result<()> {
    let bit_set = random_bit_set();
    let base = 42;
    let mut stream = BitSetDocIdStream::new(&bit_set, base);
    let mut iterator = bit_set_iterator(&bit_set, bit_set.approximate_cardinality() as i64)?;

    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(20, &mut fail_on_doc))?;
    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(100, &mut expect_next(&mut iterator, base)))?;
    assert_equals(&bit_set.prev_set_bit(99 - base), &iterator.doc_id());
    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(200, &mut expect_next(&mut iterator, base)))?;
    assert_equals(
        &bit_set.prev_set_bit(bit_set.length() as i32 - 1),
        &iterator.doc_id(),
    );
    Assert::assert_false(stream.may_have_remaining());
    Ok(())
}

fn test_count_up_to() -> Result<()> {
    let bit_set = random_bit_set();
    let base = 42;
    let mut stream = BitSetDocIdStream::new(&bit_set, base);

    Assert::assert_true(stream.may_have_remaining());
    assert_equals(&0, &stream.count_up_to(20)?);
    Assert::assert_true(stream.may_have_remaining());
    assert_equals(
        &(bit_set.cardinality_range(0, 100 - 42) as i32),
        &stream.count_up_to(100)?,
    );
    Assert::assert_true(stream.may_have_remaining());
    assert_equals(
        &(bit_set.cardinality_range(100 - 42, bit_set.length()) as i32),
        &stream.count_up_to(200)?,
    );
    Assert::assert_false(stream.may_have_remaining());
    Ok(())
}

fn test_into_array_up_to() -> Result<()> {
    let bit_set = random_bit_set();
    let mut bit_set_iterator = bit_set_iterator(&bit_set, 0)?;
    bit_set_iterator.next_doc()?;
    let base = 42;
    let mut stream = BitSetDocIdStream::new(&bit_set, base);
    let mut array = [0i32; 16];
    let mut o = array.len();
    let mut count = array.len();
    let mut up_to = 0;
    while (up_to as usize) < bit_set.length() {
        let new_up_to = (up_to + random().next_int_bound(40)).min(100);
        let mut expected = bit_set_iterator.doc_id();
        while expected < new_up_to {
            if o == count {
                count = stream.into_array_up_to(base + new_up_to, &mut array);
                o = 0;
            }
            assert_equals(&(expected + base), &array[o]);
            o += 1;
            expected = bit_set_iterator.next_doc()?;
        }
        assert_equals(&count, &o);
        assert_equals(&0, &stream.into_array_up_to(base + new_up_to, &mut array));
        up_to = new_up_to;
    }
    Ok(())
}

fn test_mix_for_each_count_up_to() -> Result<()> {
    let bit_set = random_bit_set();
    let base = 42;
    let mut stream = BitSetDocIdStream::new(&bit_set, base);
    let mut iterator = bit_set_iterator(&bit_set, bit_set.approximate_cardinality() as i64)?;

    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(20, &mut fail_on_doc))?;
    Assert::assert_true(stream.may_have_remaining());
    assert_equals(&0, &stream.count_up_to(30)?);
    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(100, &mut expect_next(&mut iterator, base)))?;
    assert_equals(&bit_set.prev_set_bit(99 - base), &iterator.doc_id());
    Assert::assert_true(stream.may_have_remaining());
    assert_equals(
        &(bit_set.cardinality_range(100 - 42, 120 - 42) as i32),
        &stream.count_up_to(120)?,
    );
    Assert::assert_true(stream.may_have_remaining());
    iterator.advance(bit_set.prev_set_bit(120 - 1 - 42))?;
    coll(stream.for_each_up_to(200, &mut expect_next(&mut iterator, base)))?;
    assert_equals(
        &bit_set.prev_set_bit(bit_set.length() as i32 - 1),
        &iterator.doc_id(),
    );
    Assert::assert_false(stream.may_have_remaining());
    Ok(())
}
