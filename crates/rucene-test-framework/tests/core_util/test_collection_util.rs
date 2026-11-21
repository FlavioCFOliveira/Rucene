//! Port of `lucene/core/src/test/org/apache/lucene/util/TestCollectionUtil.java`.
//!
//! Java's `List<Integer>` (`Arrays.asList`, `ArrayList`, `LinkedList`) is a
//! `Vec<i32>`: `CollectionUtil` sorts a random-access slice.

use std::cmp::Ordering;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::collection_util::{intro_sort, intro_sort_by, tim_sort, tim_sort_by};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestCollectionUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestCollectionUtil",
        vec![
            test("testIntroSort", test_intro_sort),
            test("testTimSort", test_tim_sort),
            test("testEmptyListSort", test_empty_list_sort),
            test("testOneElementListSort", test_one_element_list_sort),
        ],
    )
}

/// `Collections.reverseOrder()`.
fn reverse_order(a: &i32, b: &i32) -> Ordering {
    b.cmp(a)
}

/// `Collections.sort(list)`: a stable merge sort in natural order.
fn collections_sort(list: &mut [i32]) {
    list.sort();
}

/// `Collections.sort(list, Collections.reverseOrder())`.
fn collections_sort_reverse(list: &mut [i32]) {
    list.sort_by(reverse_order);
}

/// `createRandomList(maxSize)`.
fn create_random_list(max_size: i32) -> Vec<i32> {
    let rnd = random();
    let len = rnd.next_int_bound(max_size) + 1;
    let mut a: Vec<i32> = vec![0; len as usize];
    for i in 0..a.len() {
        a[i] = rnd.next_int_bound(len);
    }
    a
}

fn test_intro_sort() -> Result<()> {
    let c = LuceneTestCase::at_least(100);
    for _ in 0..c {
        let mut list1 = create_random_list(2000);
        let mut list2 = list1.clone();
        intro_sort(&mut list1);
        collections_sort(&mut list2);
        assert_equals(&list2, &list1);

        list1 = create_random_list(2000);
        list2 = list1.clone();
        intro_sort_by(&mut list1, reverse_order);
        collections_sort_reverse(&mut list2);
        assert_equals(&list2, &list1);
        // reverse back, so we can test that completely backwards sorted array (worst case) is
        // working:
        intro_sort(&mut list1);
        collections_sort(&mut list2);
        assert_equals(&list2, &list1);
    }
    Ok(())
}

fn test_tim_sort() -> Result<()> {
    let c = LuceneTestCase::at_least(100);
    for _ in 0..c {
        let mut list1 = create_random_list(2000);
        let mut list2 = list1.clone();
        tim_sort(&mut list1);
        collections_sort(&mut list2);
        assert_equals(&list2, &list1);

        list1 = create_random_list(2000);
        list2 = list1.clone();
        tim_sort_by(&mut list1, reverse_order);
        collections_sort_reverse(&mut list2);
        assert_equals(&list2, &list1);
        // reverse back, so we can test that completely backwards sorted array (worst case) is
        // working:
        tim_sort(&mut list1);
        collections_sort(&mut list2);
        assert_equals(&list2, &list1);
    }
    Ok(())
}

fn test_empty_list_sort() -> Result<()> {
    // should produce no exceptions
    let mut list: Vec<i32> = Vec::new(); // LUCENE-2989
    intro_sort(&mut list);
    tim_sort(&mut list);
    intro_sort_by(&mut list, reverse_order);
    tim_sort_by(&mut list, reverse_order);

    // check that empty non-random access lists pass sorting without ex (as sorting is not needed)
    list = Vec::new();
    intro_sort(&mut list);
    tim_sort(&mut list);
    intro_sort_by(&mut list, reverse_order);
    tim_sort_by(&mut list, reverse_order);
    Ok(())
}

fn test_one_element_list_sort() -> Result<()> {
    // check that one-element non-random access lists pass sorting without ex (as sorting is not
    // needed)
    let mut list: Vec<i32> = Vec::new();
    list.push(1);
    intro_sort(&mut list);
    tim_sort(&mut list);
    intro_sort_by(&mut list, reverse_order);
    tim_sort_by(&mut list, reverse_order);
    Ok(())
}
