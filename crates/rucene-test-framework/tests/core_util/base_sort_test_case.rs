//! Port of `lucene/core/src/test/org/apache/lucene/util/BaseSortTestCase.java`
//! and of its subclasses `TestInPlaceMergeSorter`, `TestIntroSorter` and
//! `TestTimSorter`,
//! whose only members are the constructor flag `stable` and `newSorter`.

use std::cmp::Ordering;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::util::sorter::{ArrayInPlaceMergeSorter, ArrayIntroSorter, ArrayTimSorter, Sorter};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random};
use crate::test_with;

/// `BaseSortTestCase.Entry`: `record Entry(int value, int ord) implements
/// Comparable<Entry>`, compared by `value` only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub value: i32,
    pub ord: i32,
}

impl Entry {
    pub fn new(value: i32, ord: i32) -> Entry {
        Entry { value, ord }
    }

    /// `compareTo(other)`: `Integer.compare(value, other.value)`.
    pub fn compare_to(&self, other: &Entry) -> Ordering {
        self.value.cmp(&other.value)
    }
}

/// The `Entry[]` a sorter works on. Java leaves the slots outside the sorted
/// range `null`.
pub type EntryArray = Vec<Option<Entry>>;

/// `Comparator.naturalOrder()` over `Entry`.
pub fn natural_order(a: &Option<Entry>, b: &Option<Entry>) -> Ordering {
    a.as_ref()
        .expect("naturalOrder over null")
        .compare_to(b.as_ref().expect("naturalOrder over null"))
}

/// `public abstract Sorter newSorter(Entry[] arr)`.
pub type NewSorter = for<'a> fn(&'a mut [Option<Entry>]) -> Box<dyn Sorter + 'a>;

/// The state of a `BaseSortTestCase` subclass: its `stable` flag and its
/// `newSorter`.
#[derive(Clone, Copy)]
pub struct BaseSortTestCase {
    stable: bool,
    new_sorter: NewSorter,
}

impl BaseSortTestCase {
    /// `BaseSortTestCase(boolean stable)`.
    pub fn new(stable: bool, new_sorter: NewSorter) -> BaseSortTestCase {
        BaseSortTestCase { stable, new_sorter }
    }

    /// `assertSorted(original, sorted)`.
    pub fn assert_sorted(&self, original: &[Entry], sorted: &[Option<Entry>]) {
        assert_equals(&original.len(), &sorted.len());
        let mut actually_sorted: Vec<Entry> = original.to_vec();
        // `Arrays.sort(Object[])` is a stable merge sort over `compareTo`.
        actually_sorted.sort_by(|a, b| a.compare_to(b));
        for i in 0..original.len() {
            let s = sorted[i].expect("sorted entry");
            assert_equals(&actually_sorted[i].value, &s.value);
            if self.stable {
                assert_equals(&actually_sorted[i].ord, &s.ord);
            }
        }
    }

    /// `test(Entry[] arr)`.
    pub fn test_array(&self, arr: &[Entry]) {
        let o = random().next_int_bound(1000) as usize;
        let mut to_sort: EntryArray =
            vec![None; o + arr.len() + random().next_int_bound(3) as usize];
        for (i, e) in arr.iter().enumerate() {
            to_sort[o + i] = Some(*e);
        }
        {
            let mut sorter = (self.new_sorter)(&mut to_sort);
            sorter.sort(o, o + arr.len());
        }
        self.assert_sorted(arr, &to_sort[o..o + arr.len()]);
    }

    /// `test(Strategy strategy, int length)`.
    pub fn test_strategy_length(&self, strategy: Strategy, length: i32) {
        let random = random();
        let mut arr: Vec<Entry> = Vec::with_capacity(length as usize);
        for i in 0..length as usize {
            strategy.set(&mut arr, length as usize, i, &*random);
        }
        self.test_array(&arr);
    }

    /// `test(Strategy strategy)`.
    pub fn test_strategy(&self, strategy: Strategy) {
        self.test_strategy_length(strategy, random().next_int_bound(20000));
    }

    /// `testEmpty()`.
    pub fn test_empty(&self) {
        self.test_array(&[]);
    }

    /// `testOne()`.
    pub fn test_one(&self) {
        self.test_strategy_length(Strategy::Random, 1);
    }

    /// `testTwo()`.
    pub fn test_two(&self) {
        self.test_strategy_length(Strategy::RandomLowCardinality, 2);
    }

    /// `testRandom()`.
    pub fn test_random(&self) {
        self.test_strategy(Strategy::Random);
    }

    /// `testRandomLowCardinality()`.
    pub fn test_random_low_cardinality(&self) {
        self.test_strategy(Strategy::RandomLowCardinality);
    }

    /// `testAscending()`.
    pub fn test_ascending(&self) {
        self.test_strategy(Strategy::Ascending);
    }

    /// `testAscendingSequences()`.
    pub fn test_ascending_sequences(&self) {
        self.test_strategy(Strategy::AscendingSequences);
    }

    /// `testDescending()`.
    pub fn test_descending(&self) {
        self.test_strategy(Strategy::Descending);
    }

    /// `testStrictlyDescending()`.
    pub fn test_strictly_descending(&self) {
        self.test_strategy(Strategy::StrictlyDescending);
    }

    /// The public `test*` methods of `BaseSortTestCase`, in declaration
    /// order, bound to this subclass.
    pub fn methods(self) -> Vec<Method> {
        vec![
            test_with("testEmpty", move || {
                self.test_empty();
                Ok(())
            }),
            test_with("testOne", move || {
                self.test_one();
                Ok(())
            }),
            test_with("testTwo", move || {
                self.test_two();
                Ok(())
            }),
            test_with("testRandom", move || {
                self.test_random();
                Ok(())
            }),
            test_with("testRandomLowCardinality", move || {
                self.test_random_low_cardinality();
                Ok(())
            }),
            test_with("testAscending", move || {
                self.test_ascending();
                Ok(())
            }),
            test_with("testAscendingSequences", move || {
                self.test_ascending_sequences();
                Ok(())
            }),
            test_with("testDescending", move || {
                self.test_descending();
                Ok(())
            }),
            test_with("testStrictlyDescending", move || {
                self.test_strictly_descending();
                Ok(())
            }),
        ]
    }
}

/// `BaseSortTestCase.Strategy`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    Random,
    RandomLowCardinality,
    RandomMediumCardinality,
    Ascending,
    Descending,
    StrictlyDescending,
    AscendingSequences,
    MostlyAscending,
}

impl Strategy {
    /// `set(arr, i, random)`: `arr` holds the `i` entries set so far and
    /// `length` is `arr.length`.
    pub fn set(self, arr: &mut Vec<Entry>, length: usize, i: usize, random: &dyn Random) {
        let i32_i = i as i32;
        let e = match self {
            Strategy::Random => Entry::new(random.next_int(), i32_i),
            Strategy::RandomLowCardinality => Entry::new(random.next_int_bound(6), i32_i),
            Strategy::RandomMediumCardinality => {
                Entry::new(random.next_int_bound((length / 2) as i32), i32_i)
            }
            Strategy::Ascending => {
                if i == 0 {
                    Entry::new(random.next_int_bound(6), 0)
                } else {
                    Entry::new(
                        arr[i - 1].value.wrapping_add(random.next_int_bound(6)),
                        i32_i,
                    )
                }
            }
            Strategy::Descending => {
                if i == 0 {
                    Entry::new(random.next_int_bound(6), 0)
                } else {
                    Entry::new(
                        arr[i - 1].value.wrapping_sub(random.next_int_bound(6)),
                        i32_i,
                    )
                }
            }
            Strategy::StrictlyDescending => {
                if i == 0 {
                    Entry::new(random.next_int_bound(6), 0)
                } else {
                    Entry::new(
                        arr[i - 1]
                            .value
                            .wrapping_sub(TestUtil::next_int(random, 1, 5)),
                        i32_i,
                    )
                }
            }
            Strategy::AscendingSequences => {
                if i == 0 {
                    Entry::new(random.next_int_bound(6), 0)
                } else {
                    Entry::new(
                        if LuceneTestCase::rarely_with(random) {
                            random.next_int_bound(1000)
                        } else {
                            arr[i - 1].value.wrapping_add(random.next_int_bound(6))
                        },
                        i32_i,
                    )
                }
            }
            Strategy::MostlyAscending => {
                if i == 0 {
                    Entry::new(random.next_int_bound(6), 0)
                } else {
                    Entry::new(
                        arr[i - 1]
                            .value
                            .wrapping_add(TestUtil::next_int(random, -8, 10)),
                        i32_i,
                    )
                }
            }
        };
        arr.push(e);
    }
}

/// `TestInPlaceMergeSorter`: `super(true)`; `newSorter` returns an
/// `ArrayInPlaceMergeSorter` in natural order.
pub fn in_place_merge_sorter_class_def() -> Arc<Class> {
    fn new_sorter<'a>(arr: &'a mut [Option<Entry>]) -> Box<dyn Sorter + 'a> {
        Box::new(ArrayInPlaceMergeSorter::new(arr, natural_order))
    }
    class(
        "org.apache.lucene.util.TestInPlaceMergeSorter",
        BaseSortTestCase::new(true, new_sorter).methods(),
    )
}

/// `TestIntroSorter`: `super(false)`; `newSorter` returns an
/// `ArrayIntroSorter` in natural order.
pub fn intro_sorter_class_def() -> Arc<Class> {
    fn new_sorter<'a>(arr: &'a mut [Option<Entry>]) -> Box<dyn Sorter + 'a> {
        Box::new(ArrayIntroSorter::new(arr, natural_order))
    }
    class(
        "org.apache.lucene.util.TestIntroSorter",
        BaseSortTestCase::new(false, new_sorter).methods(),
    )
}

/// `TestTimSorter`: `super(true)`; `newSorter` returns an `ArrayTimSorter` in
/// natural order with `TestUtil.nextInt(random(), 0, arr.length)` temporary
/// slots.
pub fn tim_sorter_class_def() -> Arc<Class> {
    fn new_sorter<'a>(arr: &'a mut [Option<Entry>]) -> Box<dyn Sorter + 'a> {
        let max_temp_slots = TestUtil::next_int(&*random(), 0, arr.len() as i32) as usize;
        Box::new(ArrayTimSorter::new(arr, natural_order, max_temp_slots))
    }
    class(
        "org.apache.lucene.util.TestTimSorter",
        BaseSortTestCase::new(true, new_sorter).methods(),
    )
}
