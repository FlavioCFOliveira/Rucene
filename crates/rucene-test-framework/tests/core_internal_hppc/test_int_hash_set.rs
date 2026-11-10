//! Port of `org.apache.lucene.internal.hppc.TestIntHashSet`: tests for
//! `IntHashSet`, "mostly forked and trimmed from
//! com.carrotsearch.hppc.IntHashSetTest" (HPPC 0.9.0).

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{Assert, Before};
use randomizedtesting::RandomizedTest;
use rucene::internal::hppc::hash_containers::DEFAULT_LOAD_FACTOR;
use rucene::internal::hppc::IntHashSet;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use super::support::{
    assert_array_equals, assert_equals_obj, assert_that_is, assert_that_is_empty,
    assert_that_is_greater_than_or_equal_to, assert_that_is_less_than, assert_that_is_not, random,
    CountAllocations, JavaSort, SkipLoadFactorCheckCountAllocations,
};
use crate::lucene_core_shims::ti;

const EMPTY_KEY: i32 = 0;

/// The instance state of `TestIntHashSet`.
pub struct TestIntHashSet {
    key_e: i32,
    key1: i32,
    key2: i32,
    key3: i32,
    key4: i32,

    /// Per-test fresh initialized instance.
    set: IntHashSet,
}

impl TestIntHashSet {
    fn new() -> TestIntHashSet {
        TestIntHashSet {
            key_e: 0,
            key1: cast(1),
            key2: cast(2),
            key3: cast(3),
            key4: cast(4),
            // Assigned by `initialize()` before each test.
            set: IntHashSet::new(),
        }
    }
}

/// Convert to target type from an integer used to test stuff.
fn cast(v: i32) -> i32 {
    v
}

/// `initialize()` (`@Before`).
fn initialize(this: &mut T) {
    this.set = IntHashSet::new();
}

/// `class Sub extends IntHashSet {}`: a class of its own, so that
/// `IntHashSet.equals` (`getClass() == obj.getClass()`) tells it from an
/// `IntHashSet` with the same keys.
#[derive(Debug, PartialEq)]
struct Sub(IntHashSet);

type T = TestIntHashSet;

impl TestIntHashSet {
    fn test_add_all_via_interface(&mut self) -> rucene::error::Result<()> {
        self.set.add_all_array(&[self.key1, self.key2]);
        let mut iface = IntHashSet::new();
        iface.clear();
        iface.add_all_set(&self.set);
        assert_that_is(&set(&iface.to_array()), &set(&[self.key1, self.key2]));
        Ok(())
    }

    fn test_index_methods(&mut self) -> rucene::error::Result<()> {
        let set = &mut self.set;
        set.add(self.key_e);
        set.add(self.key1);

        assert_that_is_greater_than_or_equal_to(set.index_of(self.key_e), 0);
        assert_that_is_greater_than_or_equal_to(set.index_of(self.key1), 0);
        assert_that_is_less_than(set.index_of(self.key2), 0);

        assert_that_is(&set.index_exists(set.index_of(self.key_e)), &true);
        assert_that_is(&set.index_exists(set.index_of(self.key1)), &true);
        assert_that_is(&set.index_exists(set.index_of(self.key2)), &false);

        assert_that_is(&set.index_get(set.index_of(self.key_e)), &self.key_e);
        assert_that_is(&set.index_get(set.index_of(self.key1)), &self.key1);

        expect_throws("java.lang.AssertionError", || {
            set.index_get(set.index_of(self.key2));
            Ok(())
        });

        let i = set.index_of(self.key_e);
        assert_that_is(&set.index_replace(i, self.key_e), &self.key_e);
        let i = set.index_of(self.key1);
        assert_that_is(&set.index_replace(i, self.key1), &self.key1);

        let i = set.index_of(self.key2);
        set.index_insert(i, self.key2);
        assert_that_is(&set.index_get(set.index_of(self.key2)), &self.key2);
        assert_that_is(&set.size(), &3);

        let i = set.index_of(self.key_e);
        set.index_remove(i);
        assert_that_is(&set.size(), &2);
        let i = set.index_of(self.key2);
        set.index_remove(i);
        assert_that_is(&set.size(), &1);
        assert_that_is_less_than(set.index_of(self.key_e), 0);
        assert_that_is_greater_than_or_equal_to(set.index_of(self.key1), 0);
        assert_that_is_less_than(set.index_of(self.key2), 0);
        Ok(())
    }

    fn test_cursor_index_is_valid(&mut self) -> rucene::error::Result<()> {
        self.set.add(self.key_e);
        self.set.add(self.key1);
        self.set.add(self.key2);

        for c in &self.set {
            assert_that_is(&self.set.index_exists(c.index), &true);
            assert_that_is(&self.set.index_get(c.index), &c.value);
        }
        Ok(())
    }

    fn test_empty_key(&mut self) -> rucene::error::Result<()> {
        let mut set = IntHashSet::new();
        let mut b = set.add(EMPTY_KEY);
        assert_that_is(&b, &true);
        assert_that_is(&set.add(EMPTY_KEY), &false);
        assert_that_is(&set.size(), &1);
        assert_that_is(&set.is_empty(), &false);
        assert_that_is(&self::set(&set.to_array()), &self::set(&[EMPTY_KEY]));
        assert_that_is(&set.contains(EMPTY_KEY), &true);
        let mut index = set.index_of(EMPTY_KEY);
        assert_that_is(&set.index_exists(index), &true);
        assert_that_is(&set.index_get(index), &EMPTY_KEY);
        assert_that_is(&set.index_replace(index, EMPTY_KEY), &EMPTY_KEY);
        if random().next_boolean() {
            b = set.remove(EMPTY_KEY);
            assert_that_is(&b, &true);
        } else {
            set.index_remove(index);
        }
        assert_that_is(&set.size(), &0);
        assert_that_is(&set.is_empty(), &true);
        assert_that_is_empty(&self::set(&set.to_array()));
        assert_that_is(&set.contains(EMPTY_KEY), &false);
        index = set.index_of(EMPTY_KEY);
        assert_that_is(&set.index_exists(index), &false);

        set.index_insert(index, EMPTY_KEY);
        set.add(self.key1);
        assert_that_is(&set.size(), &2);
        assert_that_is(&set.contains(EMPTY_KEY), &true);
        index = set.index_of(EMPTY_KEY);
        assert_that_is(&set.index_exists(index), &true);
        assert_that_is(&set.index_get(index), &EMPTY_KEY);
        Ok(())
    }

    fn test_ensure_capacity(&mut self) -> rucene::error::Result<()> {
        let expands = Arc::new(AtomicI32::new(0));
        let mut set = IntHashSet::with_overrides(
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
            set.add(cast(i));
        }

        let additions = random_int_between(max, max + 5000);
        set.ensure_capacity(additions + set.size());
        let before = expands.load(Ordering::SeqCst);
        for i in 0..additions {
            set.add(cast(i));
        }
        Assert::assert_equals(before, expands.load(Ordering::SeqCst));
        Ok(())
    }

    fn test_initially_empty(&mut self) -> rucene::error::Result<()> {
        Assert::assert_equals(0, self.set.size());
        Ok(())
    }

    fn test_add(&mut self) -> rucene::error::Result<()> {
        Assert::assert_true(self.set.add(self.key1));
        Assert::assert_false(self.set.add(self.key1));
        Assert::assert_equals(1, self.set.size());
        Ok(())
    }

    fn test_add2(&mut self) -> rucene::error::Result<()> {
        self.set.add_all_array(&[self.key1, self.key1]);
        Assert::assert_equals(1, self.set.size());
        Assert::assert_equals(1, self.set.add_all_array(&[self.key1, self.key2]));
        Assert::assert_equals(2, self.set.size());
        Ok(())
    }

    fn test_add_var_args(&mut self) -> rucene::error::Result<()> {
        self.set.add_all_array(&as_array(&[0, 1, 2, 1, 0]));
        Assert::assert_equals(3, self.set.size());
        assert_sorted_list_equals(self.set.to_array(), &as_array(&[0, 1, 2]));
        Ok(())
    }

    fn test_add_all(&mut self) -> rucene::error::Result<()> {
        let mut set2 = IntHashSet::new();
        set2.add_all_array(&as_array(&[1, 2]));
        self.set.add_all_array(&as_array(&[0, 1]));

        Assert::assert_equals(1, self.set.add_all_set(&set2));
        Assert::assert_equals(0, self.set.add_all_set(&set2));

        Assert::assert_equals(3, self.set.size());
        assert_sorted_list_equals(self.set.to_array(), &as_array(&[0, 1, 2]));
        Ok(())
    }

    fn test_remove(&mut self) -> rucene::error::Result<()> {
        self.set.add_all_array(&as_array(&[0, 1, 2, 3, 4]));

        Assert::assert_true(self.set.remove(self.key2));
        Assert::assert_false(self.set.remove(self.key2));
        Assert::assert_equals(4, self.set.size());
        assert_sorted_list_equals(self.set.to_array(), &as_array(&[0, 1, 3, 4]));
        Ok(())
    }

    fn test_initial_capacity_and_growth(&mut self) -> rucene::error::Result<()> {
        for i in 0..256 {
            let mut set = IntHashSet::with_expected_elements(i);

            for j in 0..i {
                set.add(cast(j));
            }

            Assert::assert_equals(i, set.size());
        }
        Ok(())
    }

    #[allow(non_snake_case)]
    fn test_bug_HPPC73_full_capacity_get(&mut self) -> rucene::error::Result<()> {
        let reallocations = Arc::new(AtomicI32::new(0));
        let elements = 0x7F;
        self.set = IntHashSet::with_overrides(
            elements,
            1.0,
            Arc::new(SkipLoadFactorCheckCountAllocations(Arc::clone(
                &reallocations,
            ))),
        );

        let reallocations_before = reallocations.load(Ordering::SeqCst);
        Assert::assert_equals(reallocations_before, 1);
        for i in 1..=elements {
            self.set.add(cast(i));
        }

        // Non-existent key.
        let out_of_set = cast(elements + 1);
        self.set.remove(out_of_set);
        Assert::assert_false(self.set.contains(out_of_set));
        Assert::assert_equals(reallocations_before, reallocations.load(Ordering::SeqCst));

        // Should not expand because we're replacing an existing element.
        Assert::assert_false(self.set.add(self.key1));
        Assert::assert_equals(reallocations_before, reallocations.load(Ordering::SeqCst));

        // Remove from a full set.
        self.set.remove(self.key1);
        Assert::assert_equals(reallocations_before, reallocations.load(Ordering::SeqCst));
        self.set.add(self.key1);

        // Check expand on "last slot of a full map" condition.
        self.set.add(out_of_set);
        Assert::assert_equals(
            reallocations_before + 1,
            reallocations.load(Ordering::SeqCst),
        );
        Ok(())
    }

    fn test_remove_all_from_lookup_container(&mut self) -> rucene::error::Result<()> {
        self.set.add_all_array(&as_array(&[0, 1, 2, 3, 4]));

        let mut list2 = IntHashSet::new();
        list2.add_all_array(&as_array(&[1, 3, 5]));

        Assert::assert_equals(2, self.set.remove_all(&list2));
        Assert::assert_equals(3, self.set.size());
        assert_sorted_list_equals(self.set.to_array(), &as_array(&[0, 2, 4]));
        Ok(())
    }

    fn test_clear(&mut self) -> rucene::error::Result<()> {
        self.set.add_all_array(&as_array(&[1, 2, 3]));
        self.set.clear();
        Assert::assert_equals(0, self.set.size());
        Ok(())
    }

    fn test_release(&mut self) -> rucene::error::Result<()> {
        self.set.add_all_array(&as_array(&[1, 2, 3]));
        self.set.release();
        Assert::assert_equals(0, self.set.size());
        self.set.add_all_array(&as_array(&[1, 2, 3]));
        Assert::assert_equals(3, self.set.size());
        Ok(())
    }

    fn test_iterable(&mut self) -> rucene::error::Result<()> {
        self.set.add_all_array(&as_array(&[1, 2, 2, 3, 4]));
        self.set.remove(self.key2);
        Assert::assert_equals(3, self.set.size());

        let mut count = 0;
        for cursor in &self.set {
            count += 1;
            Assert::assert_true(self.set.contains(cursor.value));
        }
        Assert::assert_equals(count, self.set.size());

        self.set.clear();
        Assert::assert_false(self.set.iter().next().is_some());
        Ok(())
    }

    /// Runs random insertions/deletions/clearing and compares the results
    /// against `HashSet`.
    fn test_against_hash_set(&mut self) -> rucene::error::Result<()> {
        let rnd = RandomizedTest::get_random();
        let mut other: HashSet<i32> = HashSet::new();

        let mut size = 1000;
        while size < 20000 {
            other.clear();
            self.set.clear();

            for _ in 0..size * 20 {
                let mut key = cast(rnd.next_int_bound(size));
                if rnd.next_int_bound(50) == 0 {
                    key = 0;
                }

                if rnd.next_boolean() {
                    if rnd.next_boolean() {
                        let index = self.set.index_of(key);
                        if self.set.index_exists(index) {
                            self.set.index_replace(index, key);
                        } else {
                            self.set.index_insert(index, key);
                        }
                    } else {
                        self.set.add(key);
                    }
                    other.insert(key);

                    Assert::assert_true(self.set.contains(key));
                    Assert::assert_true(self.set.index_exists(self.set.index_of(key)));
                } else {
                    Assert::assert_equals(other.contains(&key), self.set.contains(key));
                    let removed;
                    if self.set.contains(key) && rnd.next_boolean() {
                        let index = self.set.index_of(key);
                        self.set.index_remove(index);
                        removed = true;
                    } else {
                        removed = self.set.remove(key);
                    }
                    Assert::assert_equals(other.remove(&key), removed);
                }

                Assert::assert_equals(other.len() as i32, self.set.size());
            }
            size += 4000;
        }
        Ok(())
    }

    fn test_hash_code_equals(&mut self) -> rucene::error::Result<()> {
        let l0 = IntHashSet::new();
        Assert::assert_equals(0, l0.hash_code());
        assert_equals_obj(&l0, &IntHashSet::new());

        let l1 = IntHashSet::from(&[self.key1, self.key2, self.key3]);
        let mut l2 = IntHashSet::from(&[self.key1, self.key2]);
        l2.add(self.key3);

        Assert::assert_equals(l1.hash_code(), l2.hash_code());
        assert_equals_obj(&l1, &l2);
        Ok(())
    }

    fn test_clone(&mut self) -> rucene::error::Result<()> {
        self.set.add_all_array(&as_array(&[1, 2, 3]));

        let mut cloned = self.set.clone();
        cloned.remove(self.key1);

        assert_sorted_list_equals(self.set.to_array(), &as_array(&[1, 2, 3]));
        assert_sorted_list_equals(cloned.to_array(), &as_array(&[2, 3]));
        Ok(())
    }

    fn test_equals_same_class(&mut self) -> rucene::error::Result<()> {
        let l1 = IntHashSet::from(&[self.key1, self.key2, self.key3]);
        let l2 = IntHashSet::from(&[self.key1, self.key2, self.key3]);
        let l3 = IntHashSet::from(&[self.key1, self.key2, self.key4]);

        assert_that_is(&l1, &l2);
        assert_that_is(&l1.hash_code(), &l2.hash_code());
        assert_that_is_not(&l1, &l3);
        Ok(())
    }

    fn test_equals_sub_class(&mut self) -> rucene::error::Result<()> {
        let l1 = IntHashSet::from(&[self.key1, self.key2, self.key3]);
        let mut l2 = Sub(IntHashSet::new());
        let mut l3 = Sub(IntHashSet::new());
        l2.0.add_all_set(&l1);
        l3.0.add_all_set(&l1);

        assert_that_is(&l2, &l3);
        assert_that_is_not(&l1, &l2);
        Ok(())
    }
}

/// `randomIntBetween(int min, int max)` (`private static`).
fn random_int_between(min: i32, max: i32) -> i32 {
    min + random().next_int_bound(max + 1 - min)
}

/// `set(int... elements)` (`private static`): a `java.util.HashSet` of the
/// elements.
fn set(elements: &[i32]) -> HashSet<i32> {
    let mut set = HashSet::new();
    for element in elements {
        set.insert(*element);
    }
    set
}

/// `asArray(int... elements)` (`private static`).
fn as_array(elements: &[i32]) -> Vec<i32> {
    elements.to_vec()
}

/// Check if the array's content is identical to a given sequence of elements.
fn assert_sorted_list_equals(mut array: Vec<i32>, elements: &[i32]) {
    Assert::assert_equals(elements.len(), array.len());
    JavaSort::java_sort(&mut array[..]);
    assert_array_equals(elements, &array);
}

/// The class `org.apache.lucene.internal.hppc.TestIntHashSet`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.internal.hppc.TestIntHashSet")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(T::new))
        .method(Method::new("initialize").annotate(Before).body(initialize))
        .method(ti("testAddAllViaInterface", T::test_add_all_via_interface))
        .method(ti("testIndexMethods", T::test_index_methods))
        .method(ti("testCursorIndexIsValid", T::test_cursor_index_is_valid))
        .method(ti("testEmptyKey", T::test_empty_key))
        .method(ti("testEnsureCapacity", T::test_ensure_capacity))
        .method(ti("testInitiallyEmpty", T::test_initially_empty))
        .method(ti("testAdd", T::test_add))
        .method(ti("testAdd2", T::test_add2))
        .method(ti("testAddVarArgs", T::test_add_var_args))
        .method(ti("testAddAll", T::test_add_all))
        .method(ti("testRemove", T::test_remove))
        .method(ti(
            "testInitialCapacityAndGrowth",
            T::test_initial_capacity_and_growth,
        ))
        .method(ti(
            "testBug_HPPC73_FullCapacityGet",
            T::test_bug_HPPC73_full_capacity_get,
        ))
        .method(ti(
            "testRemoveAllFromLookupContainer",
            T::test_remove_all_from_lookup_container,
        ))
        .method(ti("testClear", T::test_clear))
        .method(ti("testRelease", T::test_release))
        .method(ti("testIterable", T::test_iterable))
        .method(ti("testAgainstHashSet", T::test_against_hash_set))
        .method(ti("testHashCodeEquals", T::test_hash_code_equals))
        .method(ti("testClone", T::test_clone))
        .method(ti("testEqualsSameClass", T::test_equals_same_class))
        .method(ti("testEqualsSubClass", T::test_equals_sub_class))
        .build()
}
