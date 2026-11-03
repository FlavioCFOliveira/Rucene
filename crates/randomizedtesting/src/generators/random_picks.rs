//! Port of `com.carrotsearch.randomizedtesting.generators.RandomPicks` (randomizedtesting 2.8.1).

use crate::jdk::exception::illegal_argument;
use crate::jdk::random::Random;

/// Random selections: `RandomPicks`.
pub struct RandomPicks;

impl RandomPicks {
    /// `randomFrom(r, T[] array)` and the primitive-array overloads
    /// (`byte[]`, `short[]`, `int[]`, `char[]`, `float[]`, `long[]`,
    /// `double[]`), which pick the same way.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Can't pick a random object from an
    /// empty array.`).
    pub fn random_from<'a, R: Random + ?Sized, T>(r: &R, array: &'a [T]) -> &'a T {
        if array.is_empty() {
            illegal_argument("Can't pick a random object from an empty array.");
        }
        &array[r.next_int_bound(len_i32(array.len())) as usize]
    }

    /// `randomFrom(r, List<T> list)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Can't pick a random object from an
    /// empty list.`).
    pub fn random_from_list<'a, R: Random + ?Sized, T>(r: &R, list: &'a [T]) -> &'a T {
        if list.is_empty() {
            illegal_argument("Can't pick a random object from an empty list.");
        }
        &list[r.next_int_bound(len_i32(list.len())) as usize]
    }

    /// `randomFrom(r, Collection<T> collection)`: the `nextInt(size)`-th
    /// element in iteration order, by linear scan.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Can't pick a random object from an
    /// empty collection.`).
    pub fn random_from_collection<R, I>(r: &R, collection: I) -> I::Item
    where
        R: Random + ?Sized,
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
    {
        let mut it = collection.into_iter();
        let size = it.len();
        if size == 0 {
            illegal_argument("Can't pick a random object from an empty collection.");
        }
        let pick = r.next_int_bound(len_i32(size));
        it.nth(pick as usize)
            .expect("an element at the picked index")
    }
}

/// A Java array or collection length (`int`).
fn len_i32(len: usize) -> i32 {
    i32::try_from(len).expect("a Java array length fits an int")
}
