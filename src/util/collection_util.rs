//! Collection helpers ported from `org.apache.lucene.util.CollectionUtil`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`intro_sort_by`] | `CollectionUtil.introSort(List, Comparator)` |
//! | [`intro_sort`] | `CollectionUtil.introSort(List)` |
//! | [`tim_sort_by`] | `CollectionUtil.timSort(List, Comparator)` |
//! | [`tim_sort`] | `CollectionUtil.timSort(List)` |
//! | [`new_hash_map`] | `CollectionUtil.newHashMap(int)` |
//! | [`new_hash_set`] | `CollectionUtil.newHashSet(int)` |
//!
//! **Divergence from Lucene 10.5.0.** Java's methods take a
//! `java.util.List` and throw `IllegalArgumentException` when it is not
//! `RandomAccess`, because a linked list cannot be sorted in place efficiently.
//! Rust's equivalent of a random-access list is a slice, so these functions take
//! `&mut [T]` and the runtime check disappears: the type system already
//! guarantees random access.

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::util::sorter::{ArrayIntroSorter, ArrayTimSorter, Sorter};

/// Returns a new [`HashMap`] sized to hold `size` items without resizing.
///
/// Port of `CollectionUtil.newHashMap(int)`, which Lucene 10.5.0 marks
/// deprecated in favour of `HashMap.newHashMap`. Java over-allocates by the
/// 0.75 load factor; `HashMap::with_capacity` already guarantees room for
/// `size` items, so the capacity is passed through unchanged.
pub fn new_hash_map<K: Eq + Hash, V>(size: usize) -> HashMap<K, V> {
    HashMap::with_capacity(size)
}

/// Returns a new [`HashSet`] sized to hold `size` items without resizing.
///
/// Port of `CollectionUtil.newHashSet(int)`, which Lucene 10.5.0 marks
/// deprecated.
pub fn new_hash_set<E: Eq + Hash>(size: usize) -> HashSet<E> {
    HashSet::with_capacity(size)
}

/// Sorts `list` with the intro sort algorithm, falling back to insertion sort
/// for small ranges.
///
/// Port of `CollectionUtil.introSort(List, Comparator)`.
pub fn intro_sort_by<T, C>(list: &mut [T], comparator: C)
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    let size = list.len();
    if size <= 1 {
        return;
    }
    let mut sorter = ArrayIntroSorter::new(list, comparator);
    sorter.sort(0, size);
}

/// Sorts `list` in natural order with the intro sort algorithm.
///
/// Port of `CollectionUtil.introSort(List)`.
pub fn intro_sort<T: Ord + Clone>(list: &mut [T]) {
    intro_sort_by(list, |a, b| a.cmp(b));
}

/// Sorts `list` with the TimSort algorithm, falling back to binary sort for
/// small ranges.
///
/// Port of `CollectionUtil.timSort(List, Comparator)`, including its choice of
/// `list.size() / 64` temporary slots.
pub fn tim_sort_by<T, C>(list: &mut [T], comparator: C)
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    let size = list.len();
    if size <= 1 {
        return;
    }
    let max_temp_slots = size / 64;
    let mut sorter = ArrayTimSorter::new(list, comparator, max_temp_slots);
    sorter.sort(0, size);
}

/// Sorts `list` in natural order with the TimSort algorithm.
///
/// Port of `CollectionUtil.timSort(List)`.
pub fn tim_sort<T: Ord + Clone>(list: &mut [T]) {
    tim_sort_by(list, |a, b| a.cmp(b));
}
