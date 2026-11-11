//! Port of `lucene/core/src/test/org/apache/lucene/util/TestArrayUtil.java`.
//!
//! Java's `int` sizes are Rucene's `usize`; a negative Java size is passed
//! as its two's-complement `usize`. `assertSame` over arrays Rucene returns
//! by value compares their contents.

use std::cmp::Ordering;
use std::fmt::Debug;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::{ArrayUtil, RamUsageEstimator};
use rucene_test_framework::hamcrest::{greater_than_or_equal_to, less_than, less_than_or_equal_to};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::hamcrest::{assert_equal_to, assert_greater_than};
use crate::core_support::{
    arrays_to_string, assert_array_equals, assert_equals, class, expect_throws_panic, random, test,
    unchecked, verbose,
};

const IAE: &str = "java.lang.IllegalArgumentException";
const IOOBE: &str = "java.lang.IndexOutOfBoundsException";
const NFE: &str = "java.lang.NumberFormatException";
const AE: &str = "java.lang.AssertionError";

/// Java's `-1` where Rucene takes a `usize`.
const MINUS_ONE: usize = -1i64 as usize;

/// `TestArrayUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestArrayUtil",
        vec![
            test("testGrowth", test_growth),
            test("testMaxSize", test_max_size),
            test("testTooBig", test_too_big),
            test("testExactLimit", test_exact_limit),
            test("testInvalidElementSizes", test_invalid_element_sizes),
            test("testParseInt", test_parse_int),
            test("testIntroSort", test_intro_sort),
            test(
                "testQuickToHeapSortFallback",
                test_quick_to_heap_sort_fallback,
            ),
            test("testTimSort", test_tim_sort),
            test("testMergeSortStability", test_merge_sort_stability),
            test("testTimSortStability", test_tim_sort_stability),
            test("testEmptyArraySort", test_empty_array_sort),
            test("testSelect", test_select),
            test("testGrowExact", test_grow_exact),
            test("testGrowInRange", test_grow_in_range),
            test("testGrowInRangeFloat", test_grow_in_range_float),
            test("testCopyOfSubArray", test_copy_of_sub_array),
            test("testCompareUnsigned4", test_compare_unsigned4),
            test("testCompareUnsigned8", test_compare_unsigned8),
        ],
    )
}

/// `assertThat(actual, matcher)`.
fn assert_that<T>(actual: T, matcher: rucene_test_framework::hamcrest::OrderingComparison<T>)
where
    rucene_test_framework::hamcrest::OrderingComparison<T>:
        rucene_test_framework::hamcrest::Matcher<T>,
{
    LuceneTestCase::assert_that(&actual, &matcher);
}

/// `assertArrayEquals(expecteds, actuals, delta)` over `float[]`.
fn assert_array_equals_f32(expecteds: &[f32], actuals: &[f32], delta: f32) {
    assert_equals(&expecteds.len(), &actuals.len());
    for i in 0..expecteds.len() {
        Assert::assert_equals_float_msg(
            Some(&format!("arrays first differed at element [{i}]")),
            expecteds[i],
            actuals[i],
            delta,
        );
    }
}

/// `assertArrayEquals(expecteds, actuals, delta)` over `double[]`.
fn assert_array_equals_f64(expecteds: &[f64], actuals: &[f64], delta: f64) {
    assert_equals(&expecteds.len(), &actuals.len());
    for i in 0..expecteds.len() {
        Assert::assert_equals_double_msg(
            Some(&format!("arrays first differed at element [{i}]")),
            expecteds[i],
            actuals[i],
            delta,
        );
    }
}

// Ensure ArrayUtil.getNextSize gives linear amortized cost of realloc/copy
fn test_growth() -> Result<()> {
    let mut current_size: i32 = 0;
    let mut copy_cost: i64 = 0;

    // Make sure ArrayUtil hits Integer.MAX_VALUE, if we insist:
    while current_size as usize != ArrayUtil::MAX_ARRAY_LENGTH {
        let next_size = ArrayUtil::oversize(
            1 + current_size as usize,
            RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize,
        ) as i32;
        assert_greater_than(next_size, current_size);
        if current_size > 0 {
            copy_cost += current_size as i64;
            let copy_cost_per_element = copy_cost as f64 / current_size as f64;
            assert_that(copy_cost_per_element, less_than(10.0));
        }
        current_size = next_size;
    }
    Ok(())
}

fn test_max_size() -> Result<()> {
    // intentionally pass invalid elemSizes:
    for elem_size in 0..10 {
        assert_equals(
            &ArrayUtil::MAX_ARRAY_LENGTH,
            &ArrayUtil::oversize(ArrayUtil::MAX_ARRAY_LENGTH, elem_size),
        );
        assert_equals(
            &ArrayUtil::MAX_ARRAY_LENGTH,
            &ArrayUtil::oversize(ArrayUtil::MAX_ARRAY_LENGTH - 1, elem_size),
        );
    }
    Ok(())
}

fn test_too_big() -> Result<()> {
    expect_throws_panic(IAE, || {
        ArrayUtil::oversize(ArrayUtil::MAX_ARRAY_LENGTH + 1, 1);
    });
    Ok(())
}

fn test_exact_limit() -> Result<()> {
    assert_equals(
        &ArrayUtil::MAX_ARRAY_LENGTH,
        &ArrayUtil::oversize(ArrayUtil::MAX_ARRAY_LENGTH, 1),
    );
    Ok(())
}

fn test_invalid_element_sizes() -> Result<()> {
    let rnd = random();
    let num = LuceneTestCase::at_least(10000);
    for _ in 0..num {
        let min_target_size = rnd.next_int_bound(ArrayUtil::MAX_ARRAY_LENGTH as i32);
        let elem_size = rnd.next_int_bound(11);
        let v = ArrayUtil::oversize(min_target_size as usize, elem_size as usize) as i32;
        assert_that(v, greater_than_or_equal_to(min_target_size));
    }
    Ok(())
}

/// `parseInt(s)`.
fn parse_int(s: &str) -> Result<i32> {
    let s_chars: Vec<char> = s.chars().collect();
    let start = random().next_int_bound(5) as usize;
    let mut chars: Vec<char> =
        vec!['\0'; s_chars.len() + start + random().next_int_bound(4) as usize];
    chars[start..start + s_chars.len()].copy_from_slice(&s_chars);
    ArrayUtil::parse_int(&chars, start, s_chars.len())
}

fn test_parse_int() -> Result<()> {
    expect_throws(NFE, || parse_int(""));

    expect_throws(NFE, || parse_int("foo"));

    expect_throws(NFE, || parse_int(&i64::MAX.to_string()));

    expect_throws(NFE, || parse_int("0.34"));

    assert_equal_to(parse_int("1")?, 1);
    assert_equal_to(parse_int("-10000")?, -10000);
    assert_equal_to(parse_int("1923")?, 1923);
    assert_equal_to(parse_int("-1")?, -1);
    #[allow(clippy::disallowed_names)] // the Java test's variable name
    let foo: Vec<char> = "foo 1923 bar".chars().collect();
    assert_equal_to(ArrayUtil::parse_int(&foo, 4, 4)?, 1923);
    Ok(())
}

/// `createRandomArray(maxSize)`.
fn create_random_array(max_size: i32) -> Vec<i32> {
    let rnd = random();
    let len = rnd.next_int_bound(max_size) + 1;
    let mut a: Vec<i32> = vec![0; len as usize];
    for i in 0..a.len() {
        a[i] = rnd.next_int_bound(len);
    }
    a
}

/// `Collections.reverseOrder()`.
fn reverse_order(a: &i32, b: &i32) -> Ordering {
    b.cmp(a)
}

/// `ArrayUtil.introSort(a)`.
fn intro_sort(a: &mut [i32]) {
    let len = a.len();
    ArrayUtil::intro_sort_natural(a, 0, len);
}

/// `ArrayUtil.timSort(a)`.
fn tim_sort(a: &mut [i32]) {
    let len = a.len();
    ArrayUtil::tim_sort_natural(a, 0, len);
}

fn test_intro_sort() -> Result<()> {
    let num = LuceneTestCase::at_least(50);
    for _ in 0..num {
        let mut a1 = create_random_array(2000);
        let mut a2 = a1.clone();
        intro_sort(&mut a1);
        a2.sort();
        assert_array_equals(&a2, &a1);

        a1 = create_random_array(2000);
        a2 = a1.clone();
        ArrayUtil::intro_sort_all(&mut a1, reverse_order);
        a2.sort_by(reverse_order);
        assert_array_equals(&a2, &a1);
        // reverse back, so we can test that completely backwards sorted array (worst case) is
        // working:
        intro_sort(&mut a1);
        a2.sort();
        assert_array_equals(&a2, &a1);
    }
    Ok(())
}

/// `createSparseRandomArray(maxSize)`.
fn create_sparse_random_array(max_size: i32) -> Vec<i32> {
    let rnd = random();
    let len = rnd.next_int_bound(max_size) + 1;
    let mut a: Vec<i32> = vec![0; len as usize];
    for i in 0..a.len() {
        a[i] = rnd.next_int_bound(2);
    }
    a
}

// This is a test for LUCENE-3054 (which fails without the merge sort fall back with stack
// overflow in most cases)
fn test_quick_to_heap_sort_fallback() -> Result<()> {
    let num = LuceneTestCase::at_least(10);
    for _ in 0..num {
        let mut a1 = create_sparse_random_array(40000);
        let mut a2 = a1.clone();
        intro_sort(&mut a1);
        a2.sort();
        assert_array_equals(&a2, &a1);
    }
    Ok(())
}

fn test_tim_sort() -> Result<()> {
    let num = LuceneTestCase::at_least(50);
    for _ in 0..num {
        let mut a1 = create_random_array(2000);
        let mut a2 = a1.clone();
        tim_sort(&mut a1);
        a2.sort();
        assert_array_equals(&a2, &a1);

        a1 = create_random_array(2000);
        a2 = a1.clone();
        ArrayUtil::tim_sort_all(&mut a1, reverse_order);
        a2.sort_by(reverse_order);
        assert_array_equals(&a2, &a1);
        // reverse back, so we can test that completely backwards sorted array (worst case) is
        // working:
        tim_sort(&mut a1);
        a2.sort();
        assert_array_equals(&a2, &a1);
    }
    Ok(())
}

/// `record Item(int val, int order) implements Comparable<Item>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Item {
    val: i32,
    order: i32,
}

impl Item {
    /// `compareTo(other)`: `this.order - other.order`.
    fn compare_to(&self, other: &Item) -> Ordering {
        (self.order - other.order).cmp(&0)
    }
}

impl std::fmt::Display for Item {
    /// `toString()`: `Integer.toString(val)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.val)
    }
}

/// The body `testMergeSortStability` and `testTimSortStability` share.
fn check_tim_sort_stability() {
    let rnd = random();
    let mut items: Vec<Item> = Vec::with_capacity(100);
    for i in 0..100 {
        // half of the items have value but same order. The value of this items is sorted,
        // so they should always be in order after sorting.
        // The other half has defined order, but no (-1) value (they should appear after
        // all above, when sorted).
        let equal = rnd.next_boolean();
        items.push(Item {
            val: if equal { i + 1 } else { -1 },
            order: if equal {
                0
            } else {
                rnd.next_int_bound(1000) + 1
            },
        });
    }

    if verbose() {
        println!("Before: {}", arrays_to_string(&items));
    }
    // if you replace this with ArrayUtil.quickSort(), test should fail:
    let len = items.len();
    ArrayUtil::tim_sort(&mut items, 0, len, Item::compare_to);
    if verbose() {
        println!("Sorted: {}", arrays_to_string(&items));
    }

    let mut last = items[0];
    for i in 1..items.len() {
        let act = items[i];
        if act.order == 0 {
            // order of "equal" items should be not mixed up
            assert_greater_than(act.val, last.val);
        }
        assert_that(act.order, greater_than_or_equal_to(last.order));
        last = act;
    }
}

fn test_merge_sort_stability() -> Result<()> {
    check_tim_sort_stability();
    Ok(())
}

fn test_tim_sort_stability() -> Result<()> {
    check_tim_sort_stability();
    Ok(())
}

// should produce no exceptions
fn test_empty_array_sort() -> Result<()> {
    let mut a: Vec<i32> = Vec::new();
    intro_sort(&mut a);
    tim_sort(&mut a);
    ArrayUtil::intro_sort_all(&mut a, reverse_order);
    ArrayUtil::tim_sort_all(&mut a, reverse_order);
    Ok(())
}

fn test_select() -> Result<()> {
    for _ in 0..100 {
        do_test_select();
    }
    Ok(())
}

/// `doTestSelect()`.
fn do_test_select() {
    let from = random().next_int_bound(5);
    let to = from + TestUtil::next_int(&*random(), 1, 10000);
    let max = if random().next_boolean() {
        random().next_int_bound(100)
    } else {
        random().next_int_bound(100000)
    };
    let mut arr: Vec<i32> = vec![0; (from + to + random().next_int_bound(5)) as usize];
    for i in 0..arr.len() {
        arr[i] = TestUtil::next_int(&*random(), 0, max);
    }
    let k = TestUtil::next_int(&*random(), from, to - 1) as usize;
    let (from, to) = (from as usize, to as usize);

    let mut expected = arr.clone();
    expected[from..to].sort();

    let mut actual = arr.clone();
    ArrayUtil::select(&mut actual, from, to, k, |a: &i32, b: &i32| a.cmp(b));

    assert_equals(&expected[k], &actual[k]);
    for i in 0..actual.len() {
        if i < from || i >= to {
            // `assertThat(actual[i], sameInstance(arr[i]))`: the same boxed
            // `Integer`.
            assert_equals(&arr[i], &actual[i]);
        } else if i <= k {
            assert_that(actual[i], less_than_or_equal_to(actual[k]));
        } else {
            assert_that(actual[i], greater_than_or_equal_to(actual[k]));
        }
    }
}

/// `expectThrows(IndexOutOfBoundsException.class, () -> growExact(array,
/// random().nextInt(3)))`.
fn expect_grow_exact_ioobe<T: Copy + Default + Debug>(array: &[T]) {
    expect_throws_panic(IOOBE, || {
        ArrayUtil::grow_exact_generic(array, random().next_int_bound(3) as usize);
    });
}

fn test_grow_exact() -> Result<()> {
    assert_array_equals(
        &[1i16, 2, 3, 0],
        &ArrayUtil::grow_exact_generic(&[1i16, 2, 3], 4),
    );
    assert_array_equals(
        &[1i16, 2, 3, 0, 0],
        &ArrayUtil::grow_exact_generic(&[1i16, 2, 3], 5),
    );
    expect_grow_exact_ioobe(&[1i16, 2, 3]);

    assert_array_equals(&[1i32, 2, 3, 0], &ArrayUtil::grow_exact_int(&[1, 2, 3], 4));
    assert_array_equals(
        &[1i32, 2, 3, 0, 0],
        &ArrayUtil::grow_exact_int(&[1, 2, 3], 5),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::grow_exact_int(&[1, 2, 3], random().next_int_bound(3) as usize);
    });

    assert_array_equals(&[1i64, 2, 3, 0], &ArrayUtil::grow_exact_long(&[1, 2, 3], 4));
    assert_array_equals(
        &[1i64, 2, 3, 0, 0],
        &ArrayUtil::grow_exact_long(&[1, 2, 3], 5),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::grow_exact_long(&[1, 2, 3], random().next_int_bound(3) as usize);
    });

    assert_array_equals_f32(
        &[0.1f32, 0.2, 0.3, 0.0],
        &ArrayUtil::grow_exact_generic(&[0.1f32, 0.2, 0.3], 4),
        0.001,
    );
    assert_array_equals_f32(
        &[0.1f32, 0.2, 0.3, 0.0, 0.0],
        &ArrayUtil::grow_exact_generic(&[0.1f32, 0.2, 0.3], 5),
        0.001,
    );
    expect_grow_exact_ioobe(&[1.0f32, 2.0, 3.0]);

    assert_array_equals_f64(
        &[0.1f64, 0.2, 0.3, 0.0],
        &ArrayUtil::grow_exact_generic(&[0.1f64, 0.2, 0.3], 4),
        0.001,
    );
    assert_array_equals_f64(
        &[0.1f64, 0.2, 0.3, 0.0, 0.0],
        &ArrayUtil::grow_exact_generic(&[0.1f64, 0.2, 0.3], 5),
        0.001,
    );
    expect_grow_exact_ioobe(&[0.1f64, 0.2, 0.3]);

    assert_array_equals(&[1u8, 2, 3, 0], &ArrayUtil::grow_exact(&[1, 2, 3], 4));
    assert_array_equals(&[1u8, 2, 3, 0, 0], &ArrayUtil::grow_exact(&[1, 2, 3], 5));
    expect_throws_panic(IOOBE, || {
        ArrayUtil::grow_exact(&[1, 2, 3], random().next_int_bound(3) as usize);
    });

    assert_array_equals(
        &['a', 'b', 'c', '\0'],
        &ArrayUtil::grow_exact_generic(&['a', 'b', 'c'], 4),
    );
    assert_array_equals(
        &['a', 'b', 'c', '\0', '\0'],
        &ArrayUtil::grow_exact_generic(&['a', 'b', 'c'], 5),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::grow_exact(b"abc", random().next_int_bound(3) as usize);
    });

    // `String[]`, whose new slots are `null`.
    assert_array_equals(
        &[Some("a1"), Some("b2"), Some("c3"), None],
        &ArrayUtil::grow_exact_generic(&[Some("a1"), Some("b2"), Some("c3")], 4),
    );
    assert_array_equals(
        &[Some("a1"), Some("b2"), Some("c3"), None, None],
        &ArrayUtil::grow_exact_generic(&[Some("a1"), Some("b2"), Some("c3")], 5),
    );
    expect_grow_exact_ioobe(&[Some("a"), Some("b"), Some("c")]);
    Ok(())
}

fn test_grow_in_range() -> Result<()> {
    let array: Vec<i32> = vec![1, 2, 3];

    // If minLength is negative, maxLength does not matter
    expect_throws_panic(AE, || {
        let _ = ArrayUtil::grow_in_range_int(&array, MINUS_ONE, 4);
    });
    expect_throws_panic(AE, || {
        let _ = ArrayUtil::grow_in_range_int(&array, MINUS_ONE, 0);
    });
    expect_throws_panic(AE, || {
        let _ = ArrayUtil::grow_in_range_int(&array, MINUS_ONE, MINUS_ONE);
    });

    // If minLength > maxLength, we throw an exception
    expect_throws(IAE, || ArrayUtil::grow_in_range_int(&array, 1, 0));
    expect_throws(IAE, || ArrayUtil::grow_in_range_int(&array, 4, 3));
    expect_throws(IAE, || ArrayUtil::grow_in_range_int(&array, 5, 4));

    // If minLength is sufficient, we return the array
    assert_equals(
        &array,
        &unchecked(ArrayUtil::grow_in_range_int(&array, 1, 4)),
    );
    assert_equals(
        &array,
        &unchecked(ArrayUtil::grow_in_range_int(&array, 1, 2)),
    );
    assert_equals(
        &array,
        &unchecked(ArrayUtil::grow_in_range_int(&array, 1, 1)),
    );

    let min_length = 4;
    let max_length = i32::MAX as usize;

    // The array grows normally if maxLength permits
    assert_equals(
        &ArrayUtil::oversize(min_length, 4),
        &unchecked(ArrayUtil::grow_in_range_int(
            &[1, 2, 3],
            min_length,
            max_length,
        ))
        .len(),
    );

    // The array grows to maxLength if maxLength is limiting
    assert_equals(
        &min_length,
        &unchecked(ArrayUtil::grow_in_range_int(
            &[1, 2, 3],
            min_length,
            min_length,
        ))
        .len(),
    );
    Ok(())
}

fn test_grow_in_range_float() -> Result<()> {
    let array: Vec<f32> = vec![1.0, 2.0, 3.0];

    // If minLength is negative, maxLength does not matter
    expect_throws_panic(AE, || {
        let _ = ArrayUtil::grow_in_range_float(&array, MINUS_ONE, 4);
    });
    expect_throws_panic(AE, || {
        let _ = ArrayUtil::grow_in_range_float(&array, MINUS_ONE, 0);
    });
    expect_throws_panic(AE, || {
        let _ = ArrayUtil::grow_in_range_float(&array, MINUS_ONE, MINUS_ONE);
    });

    // If minLength > maxLength, we throw an exception
    expect_throws(IAE, || ArrayUtil::grow_in_range_float(&array, 1, 0));
    expect_throws(IAE, || ArrayUtil::grow_in_range_float(&array, 4, 3));
    expect_throws(IAE, || ArrayUtil::grow_in_range_float(&array, 5, 4));

    // If minLength is sufficient, we return the array
    assert_equals(
        &array,
        &unchecked(ArrayUtil::grow_in_range_float(&array, 1, 4)),
    );
    assert_equals(
        &array,
        &unchecked(ArrayUtil::grow_in_range_float(&array, 1, 2)),
    );
    assert_equals(
        &array,
        &unchecked(ArrayUtil::grow_in_range_float(&array, 1, 1)),
    );

    let min_length = 4;
    let max_length = i32::MAX as usize;

    // The array grows normally if maxLength permits
    assert_equals(
        &ArrayUtil::oversize(min_length, 4),
        &unchecked(ArrayUtil::grow_in_range_float(
            &[1.0, 2.0, 3.0],
            min_length,
            max_length,
        ))
        .len(),
    );

    // The array grows to maxLength if maxLength is limiting
    assert_equals(
        &min_length,
        &unchecked(ArrayUtil::grow_in_range_float(
            &[1.0, 2.0, 3.0],
            min_length,
            min_length,
        ))
        .len(),
    );
    Ok(())
}

fn test_copy_of_sub_array() -> Result<()> {
    let short_array: [i16; 3] = [1, 2, 3];
    assert_array_equals(
        &[1i16],
        &ArrayUtil::copy_of_sub_array_generic(&short_array, 0, 1),
    );
    assert_array_equals(
        &[1i16, 2, 3],
        &ArrayUtil::copy_of_sub_array_generic(&short_array, 0, 3),
    );
    assert_equals(
        &0,
        &ArrayUtil::copy_of_sub_array_generic(&short_array, 0, 0).len(),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::copy_of_sub_array_generic(
            &short_array,
            0,
            4 + random().next_int_bound(10) as usize,
        );
    });

    let int_array: [i32; 3] = [1, 2, 3];
    assert_array_equals(
        &[1i32, 2],
        &ArrayUtil::copy_of_sub_array_int(&int_array, 0, 2),
    );
    assert_array_equals(
        &[1i32, 2, 3],
        &ArrayUtil::copy_of_sub_array_int(&int_array, 0, 3),
    );
    assert_equals(
        &0,
        &ArrayUtil::copy_of_sub_array_int(&int_array, 1, 1).len(),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::copy_of_sub_array_int(&int_array, 1, 4 + random().next_int_bound(10) as usize);
    });

    let long_array: [i64; 3] = [1, 2, 3];
    assert_array_equals(
        &[2i64],
        &ArrayUtil::copy_of_sub_array_long(&long_array, 1, 2),
    );
    assert_array_equals(
        &[1i64, 2, 3],
        &ArrayUtil::copy_of_sub_array_long(&long_array, 0, 3),
    );
    assert_equals(
        &0,
        &ArrayUtil::copy_of_sub_array_long(&long_array, 2, 2).len(),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::copy_of_sub_array_long(&long_array, 2, 4 + random().next_int_bound(10) as usize);
    });

    let float_array: [f32; 3] = [0.1, 0.2, 0.3];
    assert_array_equals_f32(
        &[0.2, 0.3],
        &ArrayUtil::copy_of_sub_array_generic(&float_array, 1, 3),
        0.001,
    );
    assert_array_equals_f32(
        &[0.1, 0.2, 0.3],
        &ArrayUtil::copy_of_sub_array_generic(&float_array, 0, 3),
        0.001,
    );
    assert_equals(
        &0,
        &ArrayUtil::copy_of_sub_array_generic(&float_array, 0, 0).len(),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::copy_of_sub_array_generic(
            &float_array,
            3,
            4 + random().next_int_bound(10) as usize,
        );
    });

    let double_array: [f64; 3] = [0.1, 0.2, 0.3];
    assert_array_equals_f64(
        &[0.3],
        &ArrayUtil::copy_of_sub_array_generic(&double_array, 2, 3),
        0.001,
    );
    assert_array_equals_f64(
        &[0.1, 0.2, 0.3],
        &ArrayUtil::copy_of_sub_array_generic(&double_array, 0, 3),
        0.001,
    );
    assert_equals(
        &0,
        &ArrayUtil::copy_of_sub_array_generic(&double_array, 1, 1).len(),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::copy_of_sub_array_generic(
            &double_array,
            0,
            4 + random().next_int_bound(10) as usize,
        );
    });

    let byte_array: [u8; 3] = [1, 2, 3];
    assert_array_equals(&[1u8], &ArrayUtil::copy_of_sub_array(&byte_array, 0, 1));
    assert_array_equals(
        &[1u8, 2, 3],
        &ArrayUtil::copy_of_sub_array(&byte_array, 0, 3),
    );
    assert_equals(&0, &ArrayUtil::copy_of_sub_array(&byte_array, 1, 1).len());
    expect_throws_panic(IOOBE, || {
        ArrayUtil::copy_of_sub_array(&byte_array, 1, 4 + random().next_int_bound(10) as usize);
    });

    let char_array: [char; 3] = ['a', 'b', 'c'];
    assert_array_equals(
        &['a', 'b'],
        &ArrayUtil::copy_of_sub_array_generic(&char_array, 0, 2),
    );
    assert_array_equals(
        &['a', 'b', 'c'],
        &ArrayUtil::copy_of_sub_array_generic(&char_array, 0, 3),
    );
    assert_equals(
        &0,
        &ArrayUtil::copy_of_sub_array_generic(&char_array, 1, 1).len(),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::copy_of_sub_array_generic(&char_array, 3, 4);
    });

    let object_array: [&str; 3] = ["a1", "b2", "c3"];
    assert_array_equals(
        &["a1"],
        &ArrayUtil::copy_of_sub_array_generic(&object_array, 0, 1),
    );
    assert_array_equals(
        &["a1", "b2", "c3"],
        &ArrayUtil::copy_of_sub_array_generic(&object_array, 0, 3),
    );
    assert_equals(
        &0,
        &ArrayUtil::copy_of_sub_array_generic(&object_array, 1, 1).len(),
    );
    expect_throws_panic(IOOBE, || {
        ArrayUtil::copy_of_sub_array_generic(&object_array, 2, 5);
    });
    Ok(())
}

/// `Arrays.compareUnsigned(a, aFrom, aTo, b, bFrom, bTo)`.
fn compare_unsigned(a: &[u8], b: &[u8]) -> i32 {
    match a.cmp(b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

fn test_compare_unsigned4() -> Result<()> {
    let a_offset = TestUtil::next_int(&*random(), 0, 3) as usize;
    let mut a = vec![0u8; 4 + a_offset];
    let b_offset = TestUtil::next_int(&*random(), 0, 3) as usize;
    let mut b = vec![0u8; 4 + b_offset];

    for i in 0..4 {
        a[a_offset + i] = random().next_int_bound(1 << 8) as u8;
        loop {
            b[b_offset + i] = random().next_int_bound(1 << 8) as u8;
            if b[b_offset + i] != a[a_offset + i] {
                break;
            }
        }
    }

    for i in 0..4 {
        let expected = compare_unsigned(&a[a_offset..a_offset + 4], &b[b_offset..b_offset + 4]);
        let actual = ArrayUtil::compare_unsigned4(&a, a_offset, &b, b_offset);
        assert_equals(&expected.signum(), &actual.signum());
        b[b_offset + i] = a[a_offset + i];
    }

    assert_equals(
        &0,
        &ArrayUtil::compare_unsigned4(&a, a_offset, &b, b_offset),
    );
    Ok(())
}

fn test_compare_unsigned8() -> Result<()> {
    let a_offset = TestUtil::next_int(&*random(), 0, 7) as usize;
    let mut a = vec![0u8; 8 + a_offset];
    let b_offset = TestUtil::next_int(&*random(), 0, 7) as usize;
    let mut b = vec![0u8; 8 + b_offset];

    for i in 0..8 {
        a[a_offset + i] = random().next_int_bound(1 << 8) as u8;
        loop {
            b[b_offset + i] = random().next_int_bound(1 << 8) as u8;
            if b[b_offset + i] != a[a_offset + i] {
                break;
            }
        }
    }

    for i in 0..8 {
        let expected = compare_unsigned(&a[a_offset..a_offset + 8], &b[b_offset..b_offset + 8]);
        let actual = ArrayUtil::compare_unsigned8(&a, a_offset, &b, b_offset);
        assert_equals(&expected.signum(), &actual.signum());
        b[b_offset + i] = a[a_offset + i];
    }

    assert_equals(
        &0,
        &ArrayUtil::compare_unsigned8(&a, a_offset, &b, b_offset),
    );
    Ok(())
}
