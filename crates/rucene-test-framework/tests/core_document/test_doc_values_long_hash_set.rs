//! Port of `lucene/core/src/test/org/apache/lucene/document/TestDocValuesLongHashSet.java`.
//!
//! Java's `Set<Long>` is a `java.util.HashSet`: the sets the test compares
//! with `equals` are `std::collections::HashSet` (equality ignores order),
//! and `set3`, whose first iterated element the test reads, is a
//! [`JavaHashSet`] so that element is Java's.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::doc_values_queries::DocValuesLongHashSet;
use rucene::error::Result;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_not_equals as assert_not_equals_values, class, random, test, JavaHashSet,
};

/// `TestDocValuesLongHashSet`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestDocValuesLongHashSet",
        vec![
            test("testEmpty", test_empty),
            test("testOneValue", test_one_value),
            test("testTwoValues", test_two_values),
            test("testSameValue", test_same_value),
            test("testSameMissingPlaceholder", test_same_missing_placeholder),
            test("testRandom", test_random),
        ],
    )
}

/// `longHashSet.stream().boxed().collect(Collectors.toSet())`.
fn to_set(long_hash_set: &DocValuesLongHashSet) -> HashSet<i64> {
    long_hash_set.stream().collect()
}

/// Port of `TestDocValuesLongHashSet.assertEquals(Set<Long>,
/// DocValuesLongHashSet)`.
fn assert_set_equals(set1: &JavaHashSet<i64>, long_hash_set: &DocValuesLongHashSet) {
    assert_equals(&set1.size(), &long_hash_set.size());

    let set2 = to_set(long_hash_set);
    let set1_std: HashSet<i64> = set1.iter().copied().collect();
    assert_equals(&set1_std, &set2);

    if !set1.is_empty() {
        let mut set3 = JavaHashSet::from_collection(set1.iter().copied());
        let removed = *set3.iter().next().expect("not empty");
        loop {
            let next = random().next_long();
            if next != removed && set3.add(next) {
                Assert::assert_false(long_hash_set.contains(next));
                break;
            }
        }
        assert_set_not_equals(&set3, long_hash_set);
    }

    Assert::assert_true(set1.iter().all(|v| long_hash_set.contains(*v)));
}

/// Port of `TestDocValuesLongHashSet.assertNotEquals(Set<Long>,
/// DocValuesLongHashSet)`.
fn assert_set_not_equals(set1: &JavaHashSet<i64>, long_hash_set: &DocValuesLongHashSet) {
    let set2 = to_set(long_hash_set);
    let set1_std: HashSet<i64> = set1.iter().copied().collect();

    assert_not_equals_values(&set1_std, &set2);

    let mut sorted: Vec<i64> = set1.iter().copied().collect();
    sorted.sort_unstable();
    let set3 = DocValuesLongHashSet::new(&sorted);
    assert_not_equals_values(&set2, &to_set(&set3));

    Assert::assert_false(set1.iter().all(|v| long_hash_set.contains(*v)));
}

/// Port of `TestDocValuesLongHashSet.testEmpty`.
fn test_empty() -> Result<()> {
    let set1: JavaHashSet<i64> = JavaHashSet::new();
    let set2 = DocValuesLongHashSet::new(&[]);
    assert_equals(&0, &set2.size());
    assert_equals(&i64::MAX, &set2.min_value());
    assert_equals(&i64::MIN, &set2.max_value());
    assert_set_equals(&set1, &set2);
    Ok(())
}

/// Port of `TestDocValuesLongHashSet.testOneValue`.
fn test_one_value() -> Result<()> {
    let set1 = JavaHashSet::from_collection([42i64]);
    let set2 = DocValuesLongHashSet::new(&[42]);
    assert_equals(&1, &set2.size());
    assert_equals(&42, &set2.min_value());
    assert_equals(&42, &set2.max_value());
    assert_set_equals(&set1, &set2);

    let set1 = JavaHashSet::from_collection([i64::MIN]);
    let set2 = DocValuesLongHashSet::new(&[i64::MIN]);
    assert_equals(&1, &set2.size());
    assert_equals(&i64::MIN, &set2.min_value());
    assert_equals(&i64::MIN, &set2.max_value());
    assert_set_equals(&set1, &set2);
    Ok(())
}

/// Port of `TestDocValuesLongHashSet.testTwoValues`.
fn test_two_values() -> Result<()> {
    let set1 = JavaHashSet::from_collection([42i64, i64::MAX]);
    let set2 = DocValuesLongHashSet::new(&[42, i64::MAX]);
    assert_equals(&2, &set2.size());
    assert_equals(&42, &set2.min_value());
    assert_equals(&i64::MAX, &set2.max_value());
    assert_set_equals(&set1, &set2);

    let set1 = JavaHashSet::from_collection([i64::MIN, 42i64]);
    let set2 = DocValuesLongHashSet::new(&[i64::MIN, 42]);
    assert_equals(&2, &set2.size());
    assert_equals(&i64::MIN, &set2.min_value());
    assert_equals(&42, &set2.max_value());
    assert_set_equals(&set1, &set2);
    Ok(())
}

/// Port of `TestDocValuesLongHashSet.testSameValue`.
fn test_same_value() -> Result<()> {
    let set2 = DocValuesLongHashSet::new(&[42, 42]);
    assert_equals(&1, &set2.size());
    assert_equals(&42, &set2.min_value());
    assert_equals(&42, &set2.max_value());
    Ok(())
}

/// Port of `TestDocValuesLongHashSet.testSameMissingPlaceholder`.
fn test_same_missing_placeholder() -> Result<()> {
    let set2 = DocValuesLongHashSet::new(&[i64::MIN, i64::MIN]);
    assert_equals(&1, &set2.size());
    assert_equals(&i64::MIN, &set2.min_value());
    assert_equals(&i64::MIN, &set2.max_value());
    Ok(())
}

/// Port of `TestDocValuesLongHashSet.testRandom`.
fn test_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _iter in 0..iters {
        let shift = random().next_int_bound(16);
        let mut values = vec![0i64; random().next_int_bound(1 << shift) as usize];
        for i in 0..values.len() {
            if i == 0 || random().next_int_bound(10) < 9 {
                values[i] = random().next_long();
            } else {
                values[i] = values[random().next_int_bound(i as i32) as usize];
            }
        }
        if !values.is_empty() && random().next_boolean() {
            let mid = values.len() / 2;
            values[mid] = i64::MIN;
        }
        let set1 = JavaHashSet::from_collection(values.iter().copied());
        values.sort_unstable();
        let set2 = DocValuesLongHashSet::new(&values);
        assert_set_equals(&set1, &set2);
    }
    Ok(())
}
