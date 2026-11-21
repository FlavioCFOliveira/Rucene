//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMultiset.java`.
//!
//! `assertTrue(s.add(42))`: Java's `Multiset.add` always returns `true`;
//! Rucene's `Multiset::add` returns nothing (a pre-existing divergence of
//! `src/search/multiset.rs`), so the port asserts Java's constant result.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::multiset::Multiset;
use rucene::util::java_collections::JavaHashCode;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};

/// `TestMultiset`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMultiset",
        vec![
            test("testDuplicatesMatter", test_duplicates_matter),
            test("testRandom", test_random),
        ],
    )
}

/// `multiset.add(element)`, which always returns `true` in Java.
fn add_returning<T: JavaHashCode + Eq>(multiset: &mut Multiset<T>, element: T) -> bool {
    multiset.add(element);
    true
}

fn test_duplicates_matter() -> Result<()> {
    let mut s1: Multiset<i32> = Multiset::new();
    let mut s2: Multiset<i32> = Multiset::new();
    assert_equals(&s1.len(), &s2.len());
    Assert::assert_true(s1 == s2);

    Assert::assert_true(add_returning(&mut s1, 42));
    Assert::assert_true(add_returning(&mut s2, 42));
    Assert::assert_true(s1 == s2);

    s2.add(42);
    Assert::assert_false(s1 == s2);

    s1.add(43);
    s1.add(43);
    s2.add(43);
    assert_equals(&s1.len(), &s2.len());
    Assert::assert_false(s1 == s2);
    Ok(())
}

fn to_count_map<T: Hash + Eq + Clone + std::fmt::Debug>(set: &Multiset<T>) -> HashMap<T, i32> {
    let mut map = HashMap::new();
    let mut recomputed_size = 0;
    for element in set.iter() {
        add(&mut map, element.clone());
        recomputed_size += 1;
    }
    assert_equals_msg(Some(&format!("{set:?}")), &recomputed_size, &set.len());
    map
}

fn add<T: Hash + Eq>(map: &mut HashMap<T, i32>, element: T) {
    let count = map.get(&element).copied().unwrap_or(0) + 1;
    map.insert(element, count);
}

fn remove<T: Hash + Eq>(map: &mut HashMap<T, i32>, element: &T) {
    match map.get(element).copied() {
        None => {}
        Some(1) => {
            map.remove(element);
        }
        Some(count) => {
            if let Some(v) = map.get_mut(element) {
                *v = count - 1;
            }
        }
    }
}

fn test_random() -> Result<()> {
    let mut reference: HashMap<i32, i32> = HashMap::new();
    let mut multiset: Multiset<i32> = Multiset::new();
    let iters = LuceneTestCase::at_least(100);
    for _i in 0..iters {
        let value = random().next_int_bound(10);
        match random().next_int_bound(10) {
            0..=2 => {
                remove(&mut reference, &value);
                multiset.remove(&value);
            }
            3 => {
                reference.clear();
                multiset.clear();
            }
            _ => {
                add(&mut reference, value);
                multiset.add(value);
            }
        }
        assert_equals(&reference, &to_count_map(&multiset));
    }
    Ok(())
}
