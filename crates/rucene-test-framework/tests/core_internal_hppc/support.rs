//! Helpers shared by the ported `org.apache.lucene.internal.hppc` test
//! classes: the JUnit assertions they use that `Assert` does not provide,
//! and the anonymous subclasses that override the containers' protected
//! `verifyLoadFactor` and `allocateBuffers`.

use std::fmt::{Debug, Display};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::junit::Assert;
use rucene::internal::hppc::HashContainerOverrides;
use rucene_test_framework::util::LuceneTestCase;

pub use rucene_test_framework::index::base_index_file_format_test_case::assert_array_equals;

/// `random()`.
pub fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `randomIntBetween(int min, int max)` (`private static`):
/// `min + random().nextInt(max + 1 - min)`.
pub fn random_int_between(min: i32, max: i32) -> i32 {
    min + random().next_int_bound(max + 1 - min)
}

/// `assertEquals(Object expected, Object actual)` over boxed values, `None`
/// standing for `null`.
pub fn assert_equals_opt<T: PartialEq + Display>(expected: Option<T>, actual: Option<T>) {
    if expected != actual {
        let show = |v: &Option<T>| v.as_ref().map_or("null".to_string(), |v| v.to_string());
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            show(&expected),
            show(&actual)
        ));
    }
}

/// `assertNotEquals(Object unexpected, Object actual)`: fails with `Values
/// should be different. Actual: <actual>` when the two are equal.
pub fn assert_not_equals<T: PartialEq + Debug>(unexpected: &T, actual: &T) {
    if unexpected == actual {
        Assert::fail(&format!("Values should be different. Actual: {actual:?}"));
    }
}

/// `assertEquals(float expected, float actual, 0f)`.
pub fn assert_equals_f32(expected: f32, actual: f32) {
    Assert::assert_equals_float_msg(None, expected, actual, 0.0);
}

/// `assertEquals(double expected, double actual, 0d)`.
pub fn assert_equals_f64(expected: f64, actual: f64) {
    Assert::assert_equals_double_msg(None, expected, actual, 0.0);
}

/// A primitive element type of the hppc containers: `Arrays.sort` over its
/// arrays.
pub trait JavaSort: Copy + PartialEq + Display {
    /// `Arrays.sort(array)`: ascending, in the order `Integer.compare`,
    /// `Long.compare`, `Character.compare`, `Float.compare` or
    /// `Double.compare` defines.
    fn java_sort(array: &mut [Self]);
}

impl JavaSort for i32 {
    fn java_sort(array: &mut [i32]) {
        array.sort_unstable();
    }
}

impl JavaSort for i64 {
    fn java_sort(array: &mut [i64]) {
        array.sort_unstable();
    }
}

impl JavaSort for u16 {
    fn java_sort(array: &mut [u16]) {
        array.sort_unstable();
    }
}

impl JavaSort for f32 {
    fn java_sort(array: &mut [f32]) {
        array.sort_by(|a, b| a.total_cmp(b));
    }
}

impl JavaSort for f64 {
    fn java_sort(array: &mut [f64]) {
        array.sort_by(|a, b| a.total_cmp(b));
    }
}

/// `assertSortedListEquals(T[] array, T... elements)` (`private static` in
/// each class): "Check if the array's content is identical to a given
/// sequence of elements."
pub fn assert_sorted_list_equals<E: JavaSort>(mut array: Vec<E>, elements: &[E]) {
    Assert::assert_equals(elements.len(), array.len());
    E::java_sort(&mut array);
    let mut elements = elements.to_vec();
    E::java_sort(&mut elements);
    assert_array_equals(&elements, &array);
}

/// `assertSortedListEquals(T[] array, T... elements)` of the HPPC 0.10.0
/// forks, which sort only `array`.
pub fn assert_sorted_list_equals_unsorted_elements<E: JavaSort>(mut array: Vec<E>, elements: &[E]) {
    Assert::assert_equals(elements.len(), array.len());
    E::java_sort(&mut array);
    assert_array_equals(elements, &array);
}

/// The anonymous subclass `new X(…) { protected void allocateBuffers(int
/// arraySize) { super.allocateBuffers(arraySize); counter.incrementAndGet();
/// } }`.
pub struct CountAllocations(pub Arc<AtomicI32>);

impl HashContainerOverrides for CountAllocations {
    fn allocate_buffers(&self, array_size: i32, sup: &mut dyn FnMut(i32)) {
        sup(array_size);
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// The anonymous subclass of `testBug_HPPC73_FullCapacityGet`, which also
/// overrides `verifyLoadFactor`: "Skip load factor sanity range checking."
pub struct SkipLoadFactorCheckCountAllocations(pub Arc<AtomicI32>);

impl HashContainerOverrides for SkipLoadFactorCheckCountAllocations {
    fn verify_load_factor(&self, load_factor: f64, _sup: &dyn Fn(f64) -> f64) -> f64 {
        // Skip load factor sanity range checking.
        load_factor
    }

    fn allocate_buffers(&self, array_size: i32, sup: &mut dyn FnMut(i32)) {
        sup(array_size);
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// `assertNull(Object object)`: fails with `expected null, but was:<x>`.
pub fn assert_null<T: Display>(actual: Option<T>) {
    if let Some(actual) = actual {
        Assert::fail(&format!("expected null, but was:<{actual}>"));
    }
}

/// `List.toString()`: `[a, b, c]`.
pub fn list_to_string<T: Display>(list: &[T]) -> String {
    let items: Vec<String> = list.iter().map(ToString::to_string).collect();
    format!("[{}]", items.join(", "))
}

/// `assertSortedListEquals(List<Object> array, Object... elements)`
/// (`private static`): `Assert.assertEquals(Arrays.asList(elements),
/// array.stream().sorted().toList())`.
pub fn assert_sorted_object_list_equals<E: Ord + Display + Clone>(array: Vec<E>, elements: &[E]) {
    let mut sorted = array;
    sorted.sort();
    if elements != sorted.as_slice() {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            list_to_string(elements),
            list_to_string(&sorted)
        ));
    }
}

/// `MatcherAssert.assertThat(actual, Matchers.containsInAnyOrder(items))`:
/// the two hold the same elements, as many times each, in any order.
pub fn assert_contains_in_any_order<E: PartialEq + Display>(actual: &[E], items: &[E]) {
    let mut remaining: Vec<&E> = actual.iter().collect();
    let mut matched = true;
    for item in items {
        match remaining.iter().position(|e| *e == item) {
            Some(i) => {
                remaining.remove(i);
            }
            None => {
                matched = false;
                break;
            }
        }
    }
    if !matched || !remaining.is_empty() {
        Assert::fail(&format!(
            "\nExpected: iterable with items {} in any order\n     but: was {}",
            list_to_string(items),
            list_to_string(actual)
        ));
    }
}

/// `a.equals(b)` of two objects of possibly different classes: the hppc
/// containers' `equals` first requires `getClass() == obj.getClass()`, which
/// Rust spells as the two values being of one type.
pub fn java_equals<A: PartialEq + 'static, B: 'static>(a: &A, b: &B) -> bool {
    (b as &dyn std::any::Any)
        .downcast_ref::<A>()
        .is_some_and(|b| a == b)
}

/// `MatcherAssert.assertThat(actual, is(equalTo(expected)))`.
pub fn assert_that_is<A: PartialEq + Debug + 'static, B: Debug + 'static>(
    actual: &A,
    expected: &B,
) {
    if !java_equals(actual, expected) {
        Assert::fail(&format!(
            "\nExpected: is <{expected:?}>\n     but: was <{actual:?}>"
        ));
    }
}

/// `MatcherAssert.assertThat(actual, is(not(equalTo(unexpected))))`.
pub fn assert_that_is_not<A: PartialEq + Debug + 'static, B: Debug + 'static>(
    actual: &A,
    unexpected: &B,
) {
    if java_equals(actual, unexpected) {
        Assert::fail(&format!(
            "\nExpected: is not <{unexpected:?}>\n     but: was <{actual:?}>"
        ));
    }
}

/// `MatcherAssert.assertThat(actual, is(greaterThanOrEqualTo(value)))`.
pub fn assert_that_is_greater_than_or_equal_to(actual: i32, value: i32) {
    if actual < value {
        Assert::fail(&format!(
            "\nExpected: is a value equal to or greater than <{value}>\n     but: <{actual}> was less than <{value}>"
        ));
    }
}

/// `MatcherAssert.assertThat(actual, is(lessThan(value)))`.
pub fn assert_that_is_less_than(actual: i32, value: i32) {
    if actual >= value {
        let relation = if actual == value {
            "equal to"
        } else {
            "greater than"
        };
        Assert::fail(&format!(
            "\nExpected: is a value less than <{value}>\n     but: <{actual}> was {relation} <{value}>"
        ));
    }
}

/// `MatcherAssert.assertThat(collection, is(empty()))`.
pub fn assert_that_is_empty<T: Debug>(collection: &std::collections::HashSet<T>) {
    if !collection.is_empty() {
        Assert::fail(&format!(
            "\nExpected: is an empty collection\n     but: <{collection:?}>"
        ));
    }
}

/// `assertEquals(Object expected, Object actual)` over values whose
/// `toString()` is their `Debug` rendering.
pub fn assert_equals_obj<T: PartialEq + Debug>(expected: &T, actual: &T) {
    if expected != actual {
        Assert::fail(&format!("expected:<{expected:?}> but was:<{actual:?}>"));
    }
}
