//! Port of `lucene/core/src/test/org/apache/lucene/util/TestLSBRadixSorter.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::packed::PackedInts;
use rucene::util::LSBRadixSorter;
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_array_equals, class, random, test, unchecked};

/// `TestLSBRadixSorter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestLSBRadixSorter",
        vec![
            test("testEmpty", test_empty),
            test("testOne", test_one),
            test("testTwo", test_two),
            test("testSimple", test_simple),
            test("testRandom", test_random),
            test("testSorted", test_sorted),
        ],
    )
}

/// `test(LSBRadixSorter sorter, int maxLen)`.
fn test_max_len(sorter: &mut LSBRadixSorter, max_len: i32) {
    for _ in 0..10 {
        let len = TestUtil::next_int(&*random(), 0, max_len);
        let mut arr: Vec<i32> = vec![0; (len + random().next_int_bound(10)) as usize];
        let num_bits = random().next_int_bound(31);
        let max_value = (1i32 << num_bits) - 1;
        for i in 0..arr.len() {
            arr[i] = TestUtil::next_int(&*random(), 0, max_value);
        }
        test_arr(sorter, &mut arr, len as usize);
    }
}

/// `test(LSBRadixSorter sorter, int[] arr, int len)`.
fn test_arr(sorter: &mut LSBRadixSorter, arr: &mut [i32], len: usize) {
    let mut expected: Vec<i32> = arr[0..len].to_vec();
    expected.sort();

    let mut num_bits = 0;
    for i in 0..len {
        num_bits = num_bits.max(unchecked(PackedInts::bits_required(i64::from(arr[i]))));
    }

    if random().next_boolean() {
        num_bits = TestUtil::next_int(&*random(), num_bits, 32);
    }

    sorter.sort(num_bits as u32, arr, len);
    let actual: Vec<i32> = arr[0..len].to_vec();
    assert_array_equals(&expected, &actual);
}

fn test_empty() -> Result<()> {
    test_max_len(&mut LSBRadixSorter::new(), 0);
    Ok(())
}

fn test_one() -> Result<()> {
    test_max_len(&mut LSBRadixSorter::new(), 1);
    Ok(())
}

fn test_two() -> Result<()> {
    test_max_len(&mut LSBRadixSorter::new(), 2);
    Ok(())
}

fn test_simple() -> Result<()> {
    test_max_len(&mut LSBRadixSorter::new(), 100);
    Ok(())
}

fn test_random() -> Result<()> {
    test_max_len(&mut LSBRadixSorter::new(), 10000);
    Ok(())
}

fn test_sorted() -> Result<()> {
    let mut sorter = LSBRadixSorter::new();
    for _ in 0..10 {
        let mut arr: Vec<i32> = vec![0; 10000];
        let mut a = 0i32;
        for i in 0..arr.len() {
            a += random().next_int_bound(10);
            arr[i] = a;
        }
        let len = TestUtil::next_int(&*random(), 0, arr.len() as i32) as usize;
        test_arr(&mut sorter, &mut arr, len);
    }
    Ok(())
}
