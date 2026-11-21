//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestIntSet.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::util::automaton::{FrozenIntSet, IntSet, StateSet};

use crate::core_support::{assert_equals, class, hamcrest, random, test};

/// `TestIntSet`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testFreezeEqualitySmallSet", test_freeze_equality_small_set),
        test("testFreezeEqualityLargeSet", test_freeze_equality_large_set),
        test("testMapCutover", test_map_cutover),
        test("testModify", test_modify),
        test("testHashCode", test_hash_code),
    ];
    class("org.apache.lucene.util.automaton.TestIntSet", methods)
}

/// What `IntSet.equals` reads of a set: `longHashCode()` and the first
/// `size()` values of `getArray()`.
#[derive(PartialEq, Debug)]
struct IntSetView(i64, Vec<i32>);

fn view_state(s: &mut StateSet) -> IntSetView {
    let hash = s.long_hash_code();
    let size = s.size();
    IntSetView(hash, s.get_array()[..size].to_vec())
}

fn view_frozen(s: &FrozenIntSet) -> IntSetView {
    IntSetView(s.long_hash_code(), s.get_array()[..s.size()].to_vec())
}

/// `IntSet.hashCode()`: `Long.hashCode(longHashCode())`.
fn hash_code(s: &mut StateSet) -> i32 {
    let value = s.long_hash_code();
    (value ^ ((value as u64) >> 32) as i64) as i32
}

/// `assertEquals(message, a, b)` over two `IntSet`s (`IntSet.equals`).
fn assert_int_sets_equal(message: &str, a: &IntSetView, b: &IntSetView) {
    crate::core_support::assert_equals_msg(Some(message), a, b);
}

fn test_freeze_equality_small_set() -> Result<()> {
    test_freeze_equality(10);
    Ok(())
}

fn test_freeze_equality_large_set() -> Result<()> {
    test_freeze_equality(100);
    Ok(())
}

/// `testFreezeEquality(size)`.
fn test_freeze_equality(size: i32) {
    let mut state_set = StateSet::new(0);

    for i in 0..size {
        // Some duplicates is nice but not critical
        state_set.incr(random().next_int_bound(i + 1));
    }

    let frozen0 = state_set.freeze(0);

    assert_int_sets_equal(
        "Frozen set not equal to origin sorted set.",
        &view_state(&mut state_set),
        &view_frozen(&frozen0),
    );
    assert_int_sets_equal(
        "Symmetry: Sorted set not equal to frozen set.",
        &view_frozen(&frozen0),
        &view_state(&mut state_set),
    );

    let frozen1 = state_set.freeze(random().next_int());
    assert_int_sets_equal(
        "Sorted set modified while freezing?",
        &view_state(&mut state_set),
        &view_frozen(&frozen1),
    );
    assert_int_sets_equal(
        "Frozen sets were not equal",
        &view_frozen(&frozen0),
        &view_frozen(&frozen1),
    );
}

fn test_map_cutover() -> Result<()> {
    let mut set = StateSet::new(10);
    for i in 0..35 {
        // No duplicates so there are enough elements to trigger impl cutover
        set.incr(i);
    }

    hamcrest::assert_greater_than(set.size(), 32);

    for i in 0..35 {
        // This is pretty much the worst case, perf wise
        set.decr(i);
    }

    hamcrest::assert_equal_to(set.size(), 0);
    Ok(())
}

fn test_modify() -> Result<()> {
    let mut set = StateSet::new(2);
    set.incr(1);
    set.incr(2);

    let set2 = set.freeze(0);
    assert_equals(&view_state(&mut set), &view_frozen(&set2));

    set.incr(1);
    assert_equals(&view_state(&mut set), &view_frozen(&set2));

    set.decr(1);
    assert_equals(&view_state(&mut set), &view_frozen(&set2));

    set.decr(1);
    crate::core_support::assert_not_equals(&view_state(&mut set), &view_frozen(&set2));
    Ok(())
}

fn test_hash_code() -> Result<()> {
    let mut set = StateSet::new(1000);
    let mut set2 = StateSet::new(100);
    for i in 0..100 {
        set.incr(i);
        set2.incr(99 - i);
    }
    assert_equals(&hash_code(&mut set), &hash_code(&mut set2));
    Ok(())
}
