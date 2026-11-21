//! Port of `org.apache.lucene.internal.hppc.TestIntObjectHashMap`: tests for
//! `IntObjectHashMap`, "mostly forked and trimmed from
//! com.carrotsearch.hppc.IntObjectHashMapTest" (HPPC 0.9.0).
//!
//! The map's values are Java `Integer`s (`int` values boxed into the
//! `IntObjectHashMap<Object>`), here `i32`s.
//!
//! `testNullValue`, `testPutOverExistingKey` and `testEmptyKey` `put` Java
//! `null` values: their map holds `Option<i32>` values (user decision,
//! sprint 9).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::internal::hppc::hash_containers::DEFAULT_LOAD_FACTOR;
use rucene::internal::hppc::{IntObjectCursor, IntObjectHashMap, ObjectCursor};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use super::support::{
    assert_equals_opt, assert_not_equals, assert_null, assert_sorted_list_equals,
    assert_sorted_object_list_equals, random, random_int_between, CountAllocations,
    SkipLoadFactorCheckCountAllocations,
};
use crate::lucene_core_shims::ti;

/// The instance state of `TestIntObjectHashMap`.
pub struct TestIntObjectHashMap {
    /* Ready to use key values. */
    key_e: i32,
    key1: i32,
    key2: i32,
    key3: i32,
    key4: i32,

    value0: i32,
    value1: i32,
    value2: i32,
    value3: i32,
    value4: i32,

    /// Per-test fresh initialized instance.
    map: IntObjectHashMap<i32>,
}

impl TestIntObjectHashMap {
    fn new() -> TestIntObjectHashMap {
        TestIntObjectHashMap {
            key_e: 0,
            key1: cast(1),
            key2: cast(2),
            key3: cast(3),
            key4: cast(4),
            value0: vcast(0),
            value1: vcast(1),
            value2: vcast(2),
            value3: vcast(3),
            value4: vcast(4),
            map: new_instance(),
        }
    }
}

/// Convert to target type from an integer used to test stuff.
fn cast(v: i32) -> i32 {
    v
}

/// Create a new array of a given type and copy the arguments to this array.
fn new_array(elements: &[i32]) -> Vec<i32> {
    elements.to_vec()
}

fn new_instance() -> IntObjectHashMap<i32> {
    IntObjectHashMap::new()
}

/// Convert to target type from an integer used to test stuff.
fn vcast(value: i32) -> i32 {
    value
}

/// Create a new array of a given type and copy the arguments to this array.
fn newv_array(elements: &[i32]) -> Vec<i32> {
    elements.to_vec()
}

fn assert_same_map(c1: &IntObjectHashMap<i32>, c2: &IntObjectHashMap<i32>) {
    Assert::assert_equals(c1.size(), c2.size());

    for entry in c1 {
        Assert::assert_true(c2.contains_key(entry.key));
        assert_equals_opt(Some(*entry.value), c2.get(entry.key).copied());
    }
}

/// The cursors `map.putAll(other)` iterates.
///
/// Java's `putAll(Iterable<? extends IntObjectCursor<? extends VType>>)`
/// takes the other map itself; Rucene's takes owned cursors, so the values
/// are copied out of `other`'s cursors.
fn cursors(other: &IntObjectHashMap<i32>) -> Vec<IntObjectCursor<i32>> {
    other
        .iter()
        .map(|c| IntObjectCursor {
            index: c.index,
            key: c.key,
            value: *c.value,
        })
        .collect()
}

/// `toList(Iterable<ObjectCursor<T>> values)` (`static`).
pub fn to_list<'a, T: Copy + 'a>(values: impl IntoIterator<Item = ObjectCursor<&'a T>>) -> Vec<T> {
    let mut list = Vec::new();
    for c in values {
        list.push(*c.value);
    }
    list
}

type T = TestIntObjectHashMap;

impl TestIntObjectHashMap {
    /* */
    fn test_ensure_capacity(&mut self) -> rucene::error::Result<()> {
        let expands = Arc::new(AtomicI32::new(0));
        let mut map: IntObjectHashMap<i32> = IntObjectHashMap::with_overrides(
            0,
            DEFAULT_LOAD_FACTOR as f64,
            Arc::new(CountAllocations(Arc::clone(&expands))),
        );

        // Add some elements.
        let max = if LuceneTestCase::rarely() {
            0
        } else {
            random_int_between(0, 250)
        };
        for i in 0..max {
            map.put(cast(i), self.value0);
        }

        let additions = random_int_between(max, max + 5000);
        map.ensure_capacity(additions + map.size());
        let before = expands.load(Ordering::SeqCst);
        for i in 0..additions {
            map.put(cast(i), self.value0);
        }
        Assert::assert_equals(before, expands.load(Ordering::SeqCst));
        Ok(())
    }

    fn test_cursor_index_is_valid(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key_e, self.value1);
        self.map.put(self.key1, self.value2);
        self.map.put(self.key2, self.value3);

        for c in &self.map {
            Assert::assert_true(self.map.index_exists(c.index));
            Assert::assert_equals(*c.value, *self.map.index_get(c.index));
        }
        Ok(())
    }

    fn test_index_methods(&mut self) -> rucene::error::Result<()> {
        let map = &mut self.map;
        map.put(self.key_e, self.value1);
        map.put(self.key1, self.value2);

        Assert::assert_true(map.index_of(self.key_e) >= 0);
        Assert::assert_true(map.index_of(self.key1) >= 0);
        Assert::assert_true(map.index_of(self.key2) < 0);

        Assert::assert_true(map.index_exists(map.index_of(self.key_e)));
        Assert::assert_true(map.index_exists(map.index_of(self.key1)));
        Assert::assert_false(map.index_exists(map.index_of(self.key2)));

        Assert::assert_equals(self.value1, *map.index_get(map.index_of(self.key_e)));
        Assert::assert_equals(self.value2, *map.index_get(map.index_of(self.key1)));

        expect_throws("java.lang.AssertionError", || {
            map.index_get(map.index_of(self.key2));
            Ok(())
        });

        let i = map.index_of(self.key_e);
        Assert::assert_equals(self.value1, map.index_replace(i, self.value3));
        let i = map.index_of(self.key1);
        Assert::assert_equals(self.value2, map.index_replace(i, self.value4));
        Assert::assert_equals(self.value3, *map.index_get(map.index_of(self.key_e)));
        Assert::assert_equals(self.value4, *map.index_get(map.index_of(self.key1)));

        let i = map.index_of(self.key2);
        map.index_insert(i, self.key2, self.value1);
        Assert::assert_equals(self.value1, *map.index_get(map.index_of(self.key2)));
        Assert::assert_equals(3, map.size());

        let i = map.index_of(self.key_e);
        Assert::assert_equals(self.value3, map.index_remove(i));
        Assert::assert_equals(2, map.size());
        let i = map.index_of(self.key2);
        Assert::assert_equals(self.value1, map.index_remove(i));
        Assert::assert_equals(1, map.size());
        Assert::assert_true(map.index_of(self.key_e) < 0);
        Assert::assert_true(map.index_of(self.key1) >= 0);
        Assert::assert_true(map.index_of(self.key2) < 0);
        Ok(())
    }

    /* */
    fn test_cloning_constructor(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value3);

        assert_same_map(&self.map, &IntObjectHashMap::from_map(&self.map));
        Ok(())
    }

    /* */
    fn test_from_arrays(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value3);

        let map2 = IntObjectHashMap::from(
            &new_array(&[self.key1, self.key2, self.key3]),
            newv_array(&[self.value1, self.value2, self.value3]),
        );

        assert_same_map(&self.map, &map2);
        Ok(())
    }

    fn test_get_or_default(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key2, self.value2);
        Assert::assert_true(self.map.contains_key(self.key2));

        self.map.put(self.key1, self.value1);
        Assert::assert_equals(
            self.value1,
            *self.map.get_or_default(self.key1, &self.value3),
        );
        Assert::assert_equals(
            self.value3,
            *self.map.get_or_default(self.key3, &self.value3),
        );
        self.map.remove(self.key1);
        Assert::assert_equals(
            self.value3,
            *self.map.get_or_default(self.key1, &self.value3),
        );
        Ok(())
    }

    /* */
    fn test_put(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);

        Assert::assert_true(self.map.contains_key(self.key1));
        assert_equals_opt(Some(self.value1), self.map.get(self.key1).copied());
        Ok(())
    }

    /// The per-test fresh `map` of the Java `null`-value tests: Java's
    /// `IntObjectHashMap<Object>` holds `null` values, so its values are
    /// `Option<i32>` here (`None` is `null`), and a Java `get`, `put` or
    /// `remove` answering `null` for an absent key or a `null` value is the
    /// flattened `Option`.
    fn nullable_map() -> IntObjectHashMap<Option<i32>> {
        IntObjectHashMap::new()
    }

    /* */
    fn test_null_value(&mut self) -> rucene::error::Result<()> {
        let mut map = Self::nullable_map();
        map.put(self.key1, None);

        Assert::assert_true(map.contains_key(self.key1));
        assert_null(map.get(self.key1).copied().flatten());
        Ok(())
    }

    fn test_put_over_existing_key(&mut self) -> rucene::error::Result<()> {
        let mut map = Self::nullable_map();
        map.put(self.key1, Some(self.value1));
        assert_equals_opt(
            Some(self.value1),
            map.put(self.key1, Some(self.value3)).flatten(),
        );
        assert_equals_opt(Some(self.value3), map.get(self.key1).copied().flatten());

        assert_equals_opt(Some(self.value3), map.put(self.key1, None).flatten());
        Assert::assert_true(map.contains_key(self.key1));
        assert_null(map.get(self.key1).copied().flatten());

        assert_null(map.put(self.key1, Some(self.value1)).flatten());
        assert_equals_opt(Some(self.value1), map.get(self.key1).copied().flatten());
        Ok(())
    }

    /* */
    fn test_empty_key(&mut self) -> rucene::error::Result<()> {
        let mut map = Self::nullable_map();
        let empty: i32 = 0;

        map.put(empty, Some(self.value1));
        Assert::assert_equals(1, map.size());
        Assert::assert_false(map.is_empty());
        assert_equals_opt(Some(self.value1), map.get(empty).copied().flatten());
        assert_equals_opt(
            Some(self.value1),
            *map.get_or_default(empty, &Some(self.value2)),
        );
        Assert::assert_true(map.iter().next().is_some());
        Assert::assert_equals(empty, map.iter().next().expect("hasNext").key);
        assert_equals_opt(
            Some(self.value1),
            *map.iter().next().expect("hasNext").value,
        );

        map.remove(empty);
        assert_null(map.get(empty).copied().flatten());
        Assert::assert_equals(0, map.size());

        map.put(empty, None);
        Assert::assert_equals(1, map.size());
        Assert::assert_true(map.contains_key(empty));
        assert_null(map.get(empty).copied().flatten());

        map.remove(empty);
        Assert::assert_equals(0, map.size());
        Assert::assert_false(map.contains_key(empty));
        assert_null(map.get(empty).copied().flatten());

        assert_null(map.put(empty, Some(self.value1)).flatten());
        assert_equals_opt(
            Some(self.value1),
            map.put(empty, Some(self.value2)).flatten(),
        );
        map.clear();
        Assert::assert_false(map.index_exists(map.index_of(empty)));
        assert_null(map.put(empty, Some(self.value1)).flatten());
        map.clear();
        assert_null(map.remove(empty).flatten());
        Ok(())
    }

    /* */
    fn test_put_with_expansions(&mut self) -> rucene::error::Result<()> {
        const COUNT: i32 = 10000;
        let rnd = JdkRandom::new(random().next_long());
        let mut values: HashSet<i32> = HashSet::new();

        for _ in 0..COUNT {
            let v = rnd.next_int();
            let had_key = values.contains(&cast(v));
            values.insert(cast(v));

            Assert::assert_equals(had_key, self.map.contains_key(cast(v)));
            self.map.put(cast(v), vcast(v));
            Assert::assert_equals(values.len() as i32, self.map.size());
        }
        Assert::assert_equals(values.len() as i32, self.map.size());
        Ok(())
    }

    /* */
    fn test_put_all(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value1);

        let mut map2 = new_instance();

        map2.put(self.key2, self.value2);
        map2.put(self.key_e, self.value1);

        // One new key (keyE).
        Assert::assert_equals(1, self.map.put_all(cursors(&map2)));

        // Assert the value under key2 has been replaced.
        assert_equals_opt(Some(self.value2), self.map.get(self.key2).copied());

        // And key3 has been added.
        assert_equals_opt(Some(self.value1), self.map.get(self.key_e).copied());
        Assert::assert_equals(3, self.map.size());
        Ok(())
    }

    /* */
    fn test_put_if_absent(&mut self) -> rucene::error::Result<()> {
        Assert::assert_true(self.map.put_if_absent(self.key1, self.value1));
        Assert::assert_false(self.map.put_if_absent(self.key1, self.value2));
        assert_equals_opt(Some(self.value1), self.map.get(self.key1).copied());
        Ok(())
    }

    /* */
    fn test_remove(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        assert_equals_opt(Some(self.value1), self.map.remove(self.key1));
        assert_null(self.map.remove(self.key1));
        Assert::assert_equals(0, self.map.size());

        // These are internals, but perhaps worth asserting too.
        Assert::assert_equals(0, self.map.assigned());
        Ok(())
    }

    /* */
    fn test_map_key_set(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value3);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value1);

        assert_sorted_list_equals(
            self.map.keys().to_array(),
            &[self.key1, self.key2, self.key3],
        );
        Ok(())
    }

    /* */
    fn test_map_key_set_iterator(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value3);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value1);

        let mut counted = 0;
        for c in self.map.keys() {
            Assert::assert_equals(self.map.keys[c.index as usize], c.value);
            counted += 1;
        }
        Assert::assert_equals(counted, self.map.size());
        Ok(())
    }

    /* */
    fn test_clear(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value1);
        self.map.clear();
        Assert::assert_equals(0, self.map.size());

        // These are internals, but perhaps worth asserting too.
        Assert::assert_equals(0, self.map.assigned());

        // Check values are cleared.
        assert_null(self.map.put(self.key1, self.value1));
        assert_null(self.map.remove(self.key2));
        self.map.clear();

        // Check if the map behaves properly upon subsequent use.
        self.test_put_with_expansions()
    }

    /* */
    fn test_release(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value1);
        self.map.release();
        Assert::assert_equals(0, self.map.size());

        // These are internals, but perhaps worth asserting too.
        Assert::assert_equals(0, self.map.assigned());

        // Check if the map behaves properly upon subsequent use.
        self.test_put_with_expansions()
    }

    /* */
    fn test_iterable(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value3);
        self.map.remove(self.key2);

        let mut count = 0;
        for cursor in &self.map {
            count += 1;
            Assert::assert_true(self.map.contains_key(cursor.key));
            assert_equals_opt(Some(*cursor.value), self.map.get(cursor.key).copied());

            assert_equals_opt(Some(*cursor.value), self.map.values[cursor.index as usize]);
            Assert::assert_equals(cursor.key, self.map.keys[cursor.index as usize]);
        }
        Assert::assert_equals(count, self.map.size());

        self.map.clear();
        Assert::assert_false(self.map.iter().next().is_some());
        Ok(())
    }

    /* */
    #[allow(non_snake_case)]
    fn test_bug_HPPC73_full_capacity_get(&mut self) -> rucene::error::Result<()> {
        let reallocations = Arc::new(AtomicI32::new(0));
        let elements = 0x7F;
        self.map = IntObjectHashMap::with_overrides(
            elements,
            1.0,
            Arc::new(SkipLoadFactorCheckCountAllocations(Arc::clone(
                &reallocations,
            ))),
        );

        let reallocations_before = reallocations.load(Ordering::SeqCst);
        Assert::assert_equals(reallocations_before, 1);
        for i in 1..=elements {
            self.map.put(cast(i), self.value1);
        }

        // Non-existent key.
        let out_of_set = cast(elements + 1);
        self.map.remove(out_of_set);
        Assert::assert_false(self.map.contains_key(out_of_set));
        Assert::assert_equals(reallocations_before, reallocations.load(Ordering::SeqCst));

        // Should not expand because we're replacing an existing element.
        self.map.put(self.key1, self.value2);
        Assert::assert_equals(reallocations_before, reallocations.load(Ordering::SeqCst));

        // Remove from a full map.
        self.map.remove(self.key1);
        Assert::assert_equals(reallocations_before, reallocations.load(Ordering::SeqCst));
        self.map.put(self.key1, self.value2);

        // Check expand on "last slot of a full map" condition.
        self.map.put(out_of_set, self.value1);
        Assert::assert_equals(
            reallocations_before + 1,
            reallocations.load(Ordering::SeqCst),
        );
        Ok(())
    }

    fn test_hash_code_equals(&mut self) -> rucene::error::Result<()> {
        let l0 = new_instance();
        Assert::assert_equals(0, l0.hash_code());
        Assert::assert_equals(&l0, &new_instance());

        let l1 = IntObjectHashMap::from(
            &new_array(&[self.key1, self.key2, self.key3]),
            newv_array(&[self.value1, self.value2, self.value3]),
        );

        let l2 = IntObjectHashMap::from(
            &new_array(&[self.key2, self.key1, self.key3]),
            newv_array(&[self.value2, self.value1, self.value3]),
        );

        let l3 = IntObjectHashMap::from(
            &new_array(&[self.key1, self.key2]),
            newv_array(&[self.value2, self.value1]),
        );

        Assert::assert_equals(l1.hash_code(), l2.hash_code());
        Assert::assert_equals(&l1, &l2);

        assert_not_equals(&l1, &l3);
        assert_not_equals(&l2, &l3);
        Ok(())
    }

    #[allow(non_snake_case)]
    fn test_bug_HPPC37(&mut self) -> rucene::error::Result<()> {
        let l1 = IntObjectHashMap::from(&new_array(&[self.key1]), newv_array(&[self.value1]));

        let l2 = IntObjectHashMap::from(&new_array(&[self.key2]), newv_array(&[self.value1]));

        assert_not_equals(&l1, &l2);
        assert_not_equals(&l2, &l1);
        Ok(())
    }

    /// Runs random insertions/deletions/clearing and compares the results
    /// against `HashMap`.
    fn test_against_hash_map(&mut self) -> rucene::error::Result<()> {
        let rnd = RandomizedTest::get_random();
        let mut other: HashMap<i32, i32> = HashMap::new();

        let mut size = 1000;
        while size < 20000 {
            other.clear();
            self.map.clear();

            for _ in 0..size * 20 {
                let mut key = cast(rnd.next_int_bound(size));
                if rnd.next_int_bound(50) == 0 {
                    key = 0;
                }

                let value = vcast(rnd.next_int());

                if rnd.next_boolean() {
                    let previous_value;
                    if rnd.next_boolean() {
                        let index = self.map.index_of(key);
                        if self.map.index_exists(index) {
                            previous_value = Some(self.map.index_replace(index, value));
                        } else {
                            self.map.index_insert(index, key, value);
                            previous_value = None;
                        }
                    } else {
                        previous_value = self.map.put(key, value);
                    }
                    assert_equals_opt(other.insert(key, value), previous_value);

                    assert_equals_opt(Some(value), self.map.get(key).copied());
                    Assert::assert_equals(value, *self.map.index_get(self.map.index_of(key)));
                    Assert::assert_true(self.map.contains_key(key));
                    Assert::assert_true(self.map.index_exists(self.map.index_of(key)));
                } else {
                    Assert::assert_equals(other.contains_key(&key), self.map.contains_key(key));
                    let previous_value = if self.map.contains_key(key) && rnd.next_boolean() {
                        let index = self.map.index_of(key);
                        Some(self.map.index_remove(index))
                    } else {
                        self.map.remove(key)
                    };
                    assert_equals_opt(other.remove(&key), previous_value);
                }

                Assert::assert_equals(other.len() as i32, self.map.size());
            }
            size += 4000;
        }
        Ok(())
    }

    /*
     *
     */
    fn test_clone(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value3);

        let mut cloned = self.map.clone();
        cloned.remove(self.key1);

        assert_sorted_list_equals(
            self.map.keys().to_array(),
            &[self.key1, self.key2, self.key3],
        );
        assert_sorted_list_equals(cloned.keys().to_array(), &[self.key2, self.key3]);
        Ok(())
    }

    /* */
    fn test_map_values(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value3);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value1);
        assert_sorted_object_list_equals(
            to_list(self.map.values()),
            &[self.value1, self.value2, self.value3],
        );

        self.map.clear();
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value2);
        assert_sorted_object_list_equals(
            to_list(self.map.values()),
            &[self.value1, self.value2, self.value2],
        );
        Ok(())
    }

    /* */
    fn test_map_values_iterator(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value3);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value1);

        let mut counted = 0;
        for c in self.map.values() {
            assert_equals_opt(self.map.values[c.index as usize], Some(*c.value));
            counted += 1;
        }
        Assert::assert_equals(counted, self.map.size());
        Ok(())
    }

    /* */
    fn test_equals_same_class(&mut self) -> rucene::error::Result<()> {
        let mut l1 = new_instance();
        l1.put(self.key1, self.value0);
        l1.put(self.key2, self.value1);
        l1.put(self.key3, self.value2);

        let mut l2 = IntObjectHashMap::from_map(&l1);
        l2.put_all(cursors(&l1));

        let mut l3 = IntObjectHashMap::from_map(&l2);
        l3.put_all(cursors(&l2));
        l3.put(self.key4, self.value0);

        Assert::assert_equals(&l2, &l1);
        Assert::assert_equals(l2.hash_code(), l1.hash_code());
        assert_not_equals(&l1, &l3);
        Ok(())
    }

    /* */
    /// `class Sub extends IntObjectHashMap {}`: an empty subclass, which Rust
    /// spells as the class itself.
    fn test_equals_sub_class(&mut self) -> rucene::error::Result<()> {
        let mut l1 = new_instance();
        l1.put(self.key1, self.value0);
        l1.put(self.key2, self.value1);
        l1.put(self.key3, self.value2);

        let mut l2: IntObjectHashMap<i32> = IntObjectHashMap::new(); // `new Sub()`
        l2.put_all(cursors(&l1));
        l2.put(self.key4, self.value3);

        let mut l3: IntObjectHashMap<i32> = IntObjectHashMap::new(); // `new Sub()`
        l3.put_all(cursors(&l2));

        assert_not_equals(&l1, &l2);
        Assert::assert_equals(l3.hash_code(), l2.hash_code());
        Assert::assert_equals(&l3, &l2);
        Ok(())
    }
}

/// The class `org.apache.lucene.internal.hppc.TestIntObjectHashMap`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.internal.hppc.TestIntObjectHashMap")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(T::new))
        .method(ti("testEnsureCapacity", T::test_ensure_capacity))
        .method(ti("testCursorIndexIsValid", T::test_cursor_index_is_valid))
        .method(ti("testIndexMethods", T::test_index_methods))
        .method(ti("testCloningConstructor", T::test_cloning_constructor))
        .method(ti("testFromArrays", T::test_from_arrays))
        .method(ti("testGetOrDefault", T::test_get_or_default))
        .method(ti("testPut", T::test_put))
        .method(ti("testNullValue", T::test_null_value))
        .method(ti("testPutOverExistingKey", T::test_put_over_existing_key))
        .method(ti("testPutWithExpansions", T::test_put_with_expansions))
        .method(ti("testPutAll", T::test_put_all))
        .method(ti("testPutIfAbsent", T::test_put_if_absent))
        .method(ti("testRemove", T::test_remove))
        .method(ti("testEmptyKey", T::test_empty_key))
        .method(ti("testMapKeySet", T::test_map_key_set))
        .method(ti("testMapKeySetIterator", T::test_map_key_set_iterator))
        .method(ti("testClear", T::test_clear))
        .method(ti("testRelease", T::test_release))
        .method(ti("testIterable", T::test_iterable))
        .method(ti(
            "testBug_HPPC73_FullCapacityGet",
            T::test_bug_HPPC73_full_capacity_get,
        ))
        .method(ti("testHashCodeEquals", T::test_hash_code_equals))
        .method(ti("testBug_HPPC37", T::test_bug_HPPC37))
        .method(ti("testAgainstHashMap", T::test_against_hash_map))
        .method(ti("testClone", T::test_clone))
        .method(ti("testMapValues", T::test_map_values))
        .method(ti("testMapValuesIterator", T::test_map_values_iterator))
        .method(ti("testEqualsSameClass", T::test_equals_same_class))
        .method(ti("testEqualsSubClass", T::test_equals_sub_class))
        .build()
}
