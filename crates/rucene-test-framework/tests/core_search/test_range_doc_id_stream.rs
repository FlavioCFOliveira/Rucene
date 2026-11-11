//! Port of `lucene/core/src/test/org/apache/lucene/search/TestRangeDocIdStream.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_stream::{DocIdStream, RangeDocIdStream};
use rucene::search::{CollectionError, CollectionResult};

use crate::core_support::{assert_equals, class, random, test};

/// `TestRangeDocIdStream`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestRangeDocIdStream",
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

/// A `CollectionResult` thrown as Java does.
fn coll(r: CollectionResult<()>) -> Result<()> {
    r.map_err(CollectionError::into_lucene_error)
}

/// `doc -> assertEquals(expected[0]++, doc)`.
fn expect_next(expected: &mut i32) -> impl FnMut(i32) -> CollectionResult<()> + '_ {
    move |doc| {
        assert_equals(&*expected, &doc);
        *expected += 1;
        Ok(())
    }
}

/// Port of `TestRangeDocIdStream.testForEach`.
fn test_for_each() -> Result<()> {
    let mut stream = RangeDocIdStream::new(42, 100)?;
    let mut expected = 42;
    coll(stream.for_each(&mut expect_next(&mut expected)))?;
    assert_equals(&100, &expected);
    Ok(())
}

/// Port of `TestRangeDocIdStream.testCount`.
fn test_count() -> Result<()> {
    let mut stream = RangeDocIdStream::new(42, 100)?;
    assert_equals(&(100 - 42), &stream.count()?);
    Ok(())
}

/// Port of `TestRangeDocIdStream.testIntoArray`.
fn test_into_array() -> Result<()> {
    let mut stream = RangeDocIdStream::new(42, 100)?;
    let mut array = [0i32; 16];
    let mut o = array.len();
    let mut count = array.len();
    for i in 42..100 {
        if o == count {
            count = stream.into_array(&mut array);
            o = 0;
            if 100 - i >= array.len() as i32 {
                assert_equals(&array.len(), &count);
            }
        }
        assert_equals(&i, &array[o]);
        o += 1;
    }
    assert_equals(&count, &o);
    Ok(())
}

/// Port of `TestRangeDocIdStream.testForEachUpTo`.
fn test_for_each_up_to() -> Result<()> {
    let mut stream = RangeDocIdStream::new(42, 100)?;
    let mut expected = 42;
    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(20, &mut |_doc| Assert::fail("")))?;
    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(65, &mut expect_next(&mut expected)))?;
    assert_equals(&65, &expected);
    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(120, &mut expect_next(&mut expected)))?;
    assert_equals(&100, &expected);
    Assert::assert_false(stream.may_have_remaining());
    Ok(())
}

/// Port of `TestRangeDocIdStream.testCountUpTo`.
fn test_count_up_to() -> Result<()> {
    let mut stream = RangeDocIdStream::new(42, 100)?;
    Assert::assert_true(stream.may_have_remaining());
    assert_equals(&0, &stream.count_up_to(20)?);
    Assert::assert_true(stream.may_have_remaining());
    assert_equals(&(65 - 42), &stream.count_up_to(65)?);
    Assert::assert_true(stream.may_have_remaining());
    assert_equals(&(100 - 65), &stream.count_up_to(120)?);
    Assert::assert_false(stream.may_have_remaining());
    Ok(())
}

/// Port of `TestRangeDocIdStream.testIntoArrayUpTo`.
fn test_into_array_up_to() -> Result<()> {
    let mut stream = RangeDocIdStream::new(42, 100)?;
    let mut array = [0i32; 16];
    let mut o = array.len();
    let mut count = array.len();
    let mut up_to = 42;
    while up_to < 100 {
        let new_up_to = (up_to + random().next_int_bound(40)).min(100);
        for i in up_to..new_up_to {
            if o == count {
                count = stream.into_array_up_to(new_up_to, &mut array);
                o = 0;
                if new_up_to - i >= array.len() as i32 {
                    assert_equals(&array.len(), &count);
                }
            }
            assert_equals(&i, &array[o]);
            o += 1;
        }
        assert_equals(&count, &o);
        up_to = new_up_to;
    }
    Ok(())
}

/// Port of `TestRangeDocIdStream.testMixForEachCountUpTo`.
fn test_mix_for_each_count_up_to() -> Result<()> {
    let mut stream = RangeDocIdStream::new(42, 100)?;
    let mut expected = 42;
    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(65, &mut expect_next(&mut expected)))?;
    assert_equals(&65, &expected);
    Assert::assert_true(stream.may_have_remaining());
    assert_equals(&(80 - 65), &stream.count_up_to(80)?);
    expected = 80;
    Assert::assert_true(stream.may_have_remaining());
    coll(stream.for_each_up_to(90, &mut expect_next(&mut expected)))?;
    assert_equals(&90, &expected);
    Assert::assert_true(stream.may_have_remaining());
    assert_equals(&(100 - 90), &stream.count_up_to(120)?);
    Assert::assert_false(stream.may_have_remaining());
    Ok(())
}
