//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTotalHits.java`.

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};

use crate::core_support::{assert_equals, assert_not_equals, class, test};

/// `TestTotalHits`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTotalHits",
        vec![test("testEqualsAndHashcode", test_equals_and_hashcode)],
    )
}

/// `totalHits.hashCode()`.
fn hash_code(total_hits: &TotalHits) -> u64 {
    let mut hasher = DefaultHasher::new();
    total_hits.hash(&mut hasher);
    hasher.finish()
}

/// `totalHits.equals(other)` for an `Object` argument: only a `TotalHits` can
/// be equal.
fn equals_object(total_hits: &TotalHits, other: Option<&dyn Any>) -> bool {
    other
        .and_then(|o| o.downcast_ref::<TotalHits>())
        .is_some_and(|o| o == total_hits)
}

/// Port of `TestTotalHits.testEqualsAndHashcode`.
fn test_equals_and_hashcode() -> Result<()> {
    let total_hits1 = random_total_hits()?;
    Assert::assert_false(equals_object(&total_hits1, None));
    Assert::assert_false(equals_object(&total_hits1, Some(&total_hits1.value())));
    assert_equals(&total_hits1, &total_hits1);
    assert_equals(&hash_code(&total_hits1), &hash_code(&total_hits1));

    let total_hits2 = TotalHits::new(total_hits1.value(), total_hits1.relation())?;
    assert_equals(&total_hits1, &total_hits2);
    assert_equals(&total_hits2, &total_hits1);
    assert_equals(&hash_code(&total_hits1), &hash_code(&total_hits2));

    let total_hits4 = random_total_hits()?;
    if total_hits4.value() == total_hits1.value()
        && total_hits4.relation() == total_hits1.relation()
    {
        assert_equals(&total_hits1, &total_hits4);
        assert_equals(&total_hits2, &total_hits4);
        assert_equals(&hash_code(&total_hits1), &hash_code(&total_hits4));
        assert_equals(&hash_code(&total_hits2), &hash_code(&total_hits4));
    } else {
        assert_not_equals(&total_hits1, &total_hits4);
        assert_not_equals(&total_hits2, &total_hits4);
        assert_not_equals(&hash_code(&total_hits1), &hash_code(&total_hits4));
        assert_not_equals(&hash_code(&total_hits2), &hash_code(&total_hits4));
    }
    Ok(())
}

/// `private static TotalHits randomTotalHits()`.
fn random_total_hits() -> Result<TotalHits> {
    let value = RandomizedTest::random_long_between(0, i64::MAX);
    // `TotalHits.Relation.values()`.
    let relation = *RandomizedTest::random_from(&[
        TotalHitsRelation::EQUAL_TO,
        TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
    ]);
    TotalHits::new(value, relation)
}
