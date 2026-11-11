//! Port of `org.apache.lucene.internal.hppc.TestLongIntHashMap`: tests for
//! `LongIntHashMap`, "mostly forked and trimmed from
//! com.carrotsearch.hppc.LongIntHashMapTest" (HPPC 0.9.0).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::internal::hppc::hash_containers::DEFAULT_LOAD_FACTOR;
use rucene::internal::hppc::LongIntHashMap;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use super::support::{
    assert_equals_opt, assert_not_equals, assert_sorted_list_equals, random, random_int_between,
    CountAllocations, SkipLoadFactorCheckCountAllocations,
};
use crate::lucene_core_shims::ti;

/// The instance state of `TestLongIntHashMap`.
pub struct TestLongIntHashMap {
    /* Ready to use key values. */
    key_e: i64,
    key1: i64,
    key2: i64,
    key3: i64,
    key4: i64,

    value0: i32,
    value1: i32,
    value2: i32,
    value3: i32,
    value4: i32,

    /// Per-test fresh initialized instance.
    map: LongIntHashMap,
}

impl TestLongIntHashMap {
    fn new() -> TestLongIntHashMap {
        TestLongIntHashMap {
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
fn cast(v: i32) -> i64 {
    v as i64
}

/// Create a new array of a given type and copy the arguments to this array.
fn new_array(elements: &[i64]) -> Vec<i64> {
    elements.to_vec()
}

fn new_instance() -> LongIntHashMap {
    LongIntHashMap::new()
}

/// Convert to target type from an integer used to test stuff.
fn vcast(value: i32) -> i32 {
    value
}

/// Create a new array of a given type and copy the arguments to this array.
fn newv_array(elements: &[i32]) -> Vec<i32> {
    elements.to_vec()
}

fn assert_same_map(c1: &LongIntHashMap, c2: &LongIntHashMap) {
    Assert::assert_equals(c1.size(), c2.size());

    for entry in c1 {
        Assert::assert_true(c2.contains_key(entry.key));
        Assert::assert_equals(entry.value, c2.get(entry.key));
    }
}

type T = TestLongIntHashMap;

impl TestLongIntHashMap {
    fn test_ensure_capacity(&mut self) -> rucene::error::Result<()> {
        let expands = Arc::new(AtomicI32::new(0));
        let mut map = LongIntHashMap::with_overrides(
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
            Assert::assert_equals(c.value, self.map.index_get(c.index));
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

        Assert::assert_equals(self.value1, map.index_get(map.index_of(self.key_e)));
        Assert::assert_equals(self.value2, map.index_get(map.index_of(self.key1)));

        expect_throws("java.lang.AssertionError", || {
            map.index_get(map.index_of(self.key2));
            Ok(())
        });

        let i = map.index_of(self.key_e);
        Assert::assert_equals(self.value1, map.index_replace(i, self.value3));
        let i = map.index_of(self.key1);
        Assert::assert_equals(self.value2, map.index_replace(i, self.value4));
        Assert::assert_equals(self.value3, map.index_get(map.index_of(self.key_e)));
        Assert::assert_equals(self.value4, map.index_get(map.index_of(self.key1)));

        let i = map.index_of(self.key2);
        map.index_insert(i, self.key2, self.value1);
        Assert::assert_equals(self.value1, map.index_get(map.index_of(self.key2)));
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

        assert_same_map(&self.map, &LongIntHashMap::from_map(&self.map));
        Ok(())
    }

    /* */
    fn test_from_arrays(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value3);

        let map2 = LongIntHashMap::from(
            &new_array(&[self.key1, self.key2, self.key3]),
            &newv_array(&[self.value1, self.value2, self.value3]),
        );

        assert_same_map(&self.map, &map2);
        Ok(())
    }

    fn test_get_or_default(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key2, self.value2);
        Assert::assert_true(self.map.contains_key(self.key2));

        self.map.put(self.key1, self.value1);
        Assert::assert_equals(self.value1, self.map.get_or_default(self.key1, self.value3));
        Assert::assert_equals(self.value3, self.map.get_or_default(self.key3, self.value3));
        self.map.remove(self.key1);
        Assert::assert_equals(self.value3, self.map.get_or_default(self.key1, self.value3));
        Ok(())
    }

    /* */
    fn test_put(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);

        Assert::assert_true(self.map.contains_key(self.key1));
        Assert::assert_equals(self.value1, self.map.get(self.key1));
        Ok(())
    }

    /* */
    fn test_put_over_existing_key(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        Assert::assert_equals(self.value1, self.map.put(self.key1, self.value3));
        Assert::assert_equals(self.value3, self.map.get(self.key1));
        Ok(())
    }

    /* */
    fn test_put_with_expansions(&mut self) -> rucene::error::Result<()> {
        const COUNT: i32 = 10000;
        let rnd = JdkRandom::new(random().next_long());
        let mut values: HashSet<i64> = HashSet::new();

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
        Assert::assert_equals(1, self.map.put_all(&map2));

        // Assert the value under key2 has been replaced.
        Assert::assert_equals(self.value2, self.map.get(self.key2));

        // And key3 has been added.
        Assert::assert_equals(self.value1, self.map.get(self.key_e));
        Assert::assert_equals(3, self.map.size());
        Ok(())
    }

    /* */
    fn test_put_if_absent(&mut self) -> rucene::error::Result<()> {
        Assert::assert_true(self.map.put_if_absent(self.key1, self.value1));
        Assert::assert_false(self.map.put_if_absent(self.key1, self.value2));
        Assert::assert_equals(self.value1, self.map.get(self.key1));
        Ok(())
    }

    fn test_put_or_add(&mut self) -> rucene::error::Result<()> {
        Assert::assert_equals(
            self.value1,
            self.map.put_or_add(self.key1, self.value1, self.value2),
        );
        Assert::assert_equals(
            self.value3,
            self.map.put_or_add(self.key1, self.value1, self.value2),
        );
        Ok(())
    }

    fn test_add_to(&mut self) -> rucene::error::Result<()> {
        Assert::assert_equals(self.value1, self.map.add_to(self.key1, self.value1));
        Assert::assert_equals(self.value3, self.map.add_to(self.key1, self.value2));
        Ok(())
    }

    /* */
    fn test_remove(&mut self) -> rucene::error::Result<()> {
        self.map.put(self.key1, self.value1);
        Assert::assert_equals(self.value1, self.map.remove(self.key1));
        Assert::assert_equals(0, self.map.remove(self.key1));
        Assert::assert_equals(0, self.map.size());

        // These are internals, but perhaps worth asserting too.
        Assert::assert_equals(0, self.map.assigned());
        Ok(())
    }

    /* */
    fn test_empty_key(&mut self) -> rucene::error::Result<()> {
        let empty = 0;

        self.map.put(empty, self.value1);
        Assert::assert_equals(1, self.map.size());
        Assert::assert_false(self.map.is_empty());
        Assert::assert_equals(self.value1, self.map.get(empty));
        Assert::assert_equals(self.value1, self.map.get_or_default(empty, self.value2));
        Assert::assert_true(self.map.iter().next().is_some());
        Assert::assert_equals(empty, next(&self.map).key);
        Assert::assert_equals(self.value1, next(&self.map).value);

        self.map.remove(empty);
        Assert::assert_equals(0, self.map.get(empty));
        Assert::assert_equals(0, self.map.size());

        Assert::assert_equals(0, self.map.put(empty, self.value1));
        Assert::assert_equals(self.value1, self.map.put(empty, self.value2));
        self.map.clear();
        Assert::assert_false(self.map.index_exists(self.map.index_of(empty)));
        Assert::assert_equals(0, self.map.put(empty, self.value1));
        self.map.clear();
        Assert::assert_equals(0, self.map.remove(empty));
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
        Assert::assert_equals(0, self.map.put(self.key1, self.value1));
        Assert::assert_equals(0, self.map.remove(self.key2));
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
            Assert::assert_equals(cursor.value, self.map.get(cursor.key));

            Assert::assert_equals(cursor.value, self.map.values[cursor.index as usize]);
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
        self.map = LongIntHashMap::with_overrides(
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

        let l1 = LongIntHashMap::from(
            &new_array(&[self.key1, self.key2, self.key3]),
            &newv_array(&[self.value1, self.value2, self.value3]),
        );

        let l2 = LongIntHashMap::from(
            &new_array(&[self.key2, self.key1, self.key3]),
            &newv_array(&[self.value2, self.value1, self.value3]),
        );

        let l3 = LongIntHashMap::from(
            &new_array(&[self.key1, self.key2]),
            &newv_array(&[self.value2, self.value1]),
        );

        Assert::assert_equals(l1.hash_code(), l2.hash_code());
        Assert::assert_equals(&l1, &l2);

        assert_not_equals(&l1, &l3);
        assert_not_equals(&l2, &l3);
        Ok(())
    }

    #[allow(non_snake_case)]
    fn test_bug_HPPC37(&mut self) -> rucene::error::Result<()> {
        let l1 = LongIntHashMap::from(&new_array(&[self.key1]), &newv_array(&[self.value1]));

        let l2 = LongIntHashMap::from(&new_array(&[self.key2]), &newv_array(&[self.value1]));

        assert_not_equals(&l1, &l2);
        assert_not_equals(&l2, &l1);
        Ok(())
    }

    /// Runs random insertions/deletions/clearing and compares the results
    /// against `HashMap`.
    fn test_against_hash_map(&mut self) -> rucene::error::Result<()> {
        let rnd = RandomizedTest::get_random();
        let mut other: HashMap<i64, i32> = HashMap::new();

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

                let had_old_value = self.map.contains_key(key);
                if rnd.next_boolean() {
                    let previous_value;
                    if rnd.next_boolean() {
                        let index = self.map.index_of(key);
                        if self.map.index_exists(index) {
                            previous_value = self.map.index_replace(index, value);
                        } else {
                            self.map.index_insert(index, key, value);
                            previous_value = 0;
                        }
                    } else {
                        previous_value = self.map.put(key, value);
                    }
                    assert_equals_opt(
                        other.insert(key, value),
                        if previous_value == 0 && !had_old_value {
                            None
                        } else {
                            Some(previous_value)
                        },
                    );

                    Assert::assert_equals(value, self.map.get(key));
                    Assert::assert_equals(value, self.map.index_get(self.map.index_of(key)));
                    Assert::assert_true(self.map.contains_key(key));
                    Assert::assert_true(self.map.index_exists(self.map.index_of(key)));
                } else {
                    Assert::assert_equals(other.contains_key(&key), self.map.contains_key(key));
                    let previous_value = if self.map.contains_key(key) && rnd.next_boolean() {
                        let index = self.map.index_of(key);
                        self.map.index_remove(index)
                    } else {
                        self.map.remove(key)
                    };
                    assert_equals_opt(
                        other.remove(&key),
                        if previous_value == 0 && !had_old_value {
                            None
                        } else {
                            Some(previous_value)
                        },
                    );
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
        assert_sorted_list_equals(
            self.map.values().to_array(),
            &[self.value1, self.value2, self.value3],
        );

        self.map.clear();
        self.map.put(self.key1, self.value1);
        self.map.put(self.key2, self.value2);
        self.map.put(self.key3, self.value2);
        assert_sorted_list_equals(
            self.map.values().to_array(),
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
            Assert::assert_equals(self.map.values[c.index as usize], c.value);
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

        let mut l2 = LongIntHashMap::from_map(&l1);
        l2.put_all(&l1);

        let mut l3 = LongIntHashMap::from_map(&l2);
        l3.put_all(&l2);
        l3.put(self.key4, self.value0);

        Assert::assert_equals(&l2, &l1);
        Assert::assert_equals(l2.hash_code(), l1.hash_code());
        assert_not_equals(&l1, &l3);
        Ok(())
    }

    /* */
    /// `class Sub extends LongIntHashMap {}`: an empty subclass, which Rust
    /// spells as the class itself.
    fn test_equals_sub_class(&mut self) -> rucene::error::Result<()> {
        let mut l1 = new_instance();
        l1.put(self.key1, self.value0);
        l1.put(self.key2, self.value1);
        l1.put(self.key3, self.value2);

        let mut l2 = LongIntHashMap::new(); // `new Sub()`
        l2.put_all(&l1);
        l2.put(self.key4, self.value3);

        let mut l3 = LongIntHashMap::new(); // `new Sub()`
        l3.put_all(&l2);

        assert_not_equals(&l1, &l2);
        Assert::assert_equals(l3.hash_code(), l2.hash_code());
        Assert::assert_equals(&l3, &l2);
        Ok(())
    }
}

/// `map.iterator().next()`.
fn next(map: &LongIntHashMap) -> rucene::internal::hppc::LongIntCursor {
    map.iter().next().expect("java.util.NoSuchElementException")
}

/// The class `org.apache.lucene.internal.hppc.TestLongIntHashMap`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.internal.hppc.TestLongIntHashMap")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(T::new))
        .method(ti("testEnsureCapacity", T::test_ensure_capacity))
        .method(ti("testCursorIndexIsValid", T::test_cursor_index_is_valid))
        .method(ti("testIndexMethods", T::test_index_methods))
        .method(ti("testCloningConstructor", T::test_cloning_constructor))
        .method(ti("testFromArrays", T::test_from_arrays))
        .method(ti("testGetOrDefault", T::test_get_or_default))
        .method(ti("testPut", T::test_put))
        .method(ti("testPutOverExistingKey", T::test_put_over_existing_key))
        .method(ti("testPutWithExpansions", T::test_put_with_expansions))
        .method(ti("testPutAll", T::test_put_all))
        .method(ti("testPutIfAbsent", T::test_put_if_absent))
        .method(ti("testPutOrAdd", T::test_put_or_add))
        .method(ti("testAddTo", T::test_add_to))
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
