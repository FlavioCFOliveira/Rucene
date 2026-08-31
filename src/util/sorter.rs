//! Sorting algorithms ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`Sorter`] | `Sorter` |
//! | [`InPlaceMergeSorter`] | `InPlaceMergeSorter` |
//! | [`TimSorter`] / [`TimSorterState`] | `TimSorter` |
//! | [`ArrayInPlaceMergeSorter`] | `ArrayInPlaceMergeSorter` |
//! | [`ArrayIntroSorter`] | `ArrayIntroSorter` |
//! | [`ArrayTimSorter`] | `ArrayTimSorter` |
//! | [`MSBRadixSorter`] / [`MSBRadixSorterOps`] | `MSBRadixSorter` |
//! | [`StableMSBRadixSorter`] / [`StableMSBRadixSorterOps`] | `StableMSBRadixSorter` |
//! | [`MergeSorter`] | `StableMSBRadixSorter.MergeSorter` |
//! | [`LSBRadixSorter`] | `LSBRadixSorter` |
//! | [`BytesRefComparator`] / [`NaturalBytesRefComparator`] | `BytesRefComparator` (and its `NATURAL`) |
//! | [`StringSorter`] / [`StringSorterOps`] | `StringSorter` |
//! | [`StableStringSorter`] / [`StableStringSorterOps`] | `StableStringSorter` |
//!
//! # Shape of the port
//!
//! Java expresses every sorter as an abstract class whose `compare` and `swap`
//! the caller fills in. Rust has no inheritance, so the abstract classes become
//! traits and the algorithms become default methods (or free functions) over
//! them. The four primitives `swap`, `setPivot`, `comparePivot` and `compare`
//! are already declared once by [`PivotOps`], which
//! [`crate::util::selector`] introduced for `IntroSorter`/`IntroSelector`, so
//! [`Sorter`] extends that trait rather than re-declaring them.
//!
//! Two mechanical consequences of that choice, neither of them observable:
//!
//! * Java's `Sorter` keeps the pivot slot in a private `pivotIndex` field and
//!   derives `comparePivot(j)` from it. A Rust trait cannot hold state, so an
//!   implementor that wants Java's default behaviour stores the index itself.
//!   Because [`PivotOps::compare`] has a default expressed in terms of
//!   `setPivot`/`comparePivot`, **every [`Sorter`] implementor must override
//!   `compare`**, otherwise the two defaults call each other forever.
//! * Java's `TimSorter` overrides `Sorter.doRotate`. Rust cannot override a
//!   supertrait's default method from a subtrait, so a [`TimSorter`]
//!   implementor forwards [`Sorter::do_rotate`] to
//!   [`TimSorter::tim_do_rotate`] explicitly.

#![deny(unsafe_code)]

use std::cmp::Ordering;

use crate::util::selector::{intro_sort, PivotOps};
use crate::util::{ArrayUtil, BytesRef};

/// Sub-ranges shorter than this are sorted with binary sort.
///
/// Equivalent to `Sorter.BINARY_SORT_THRESHOLD`.
pub const BINARY_SORT_THRESHOLD: usize = 20;

/// Sub-ranges shorter than this are sorted with insertion sort.
///
/// Equivalent to `Sorter.INSERTION_SORT_THRESHOLD`.
pub const INSERTION_SORT_THRESHOLD: usize = 16;

/// Maps a [`std::cmp::Ordering`] to the `int` contract of `Comparator.compare`.
pub fn ordering_to_int(o: Ordering) -> i32 {
    match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

// ---------------------------------------------------------------------------
// Sorter
// ---------------------------------------------------------------------------

/// Base class for sorting algorithm implementations.
///
/// Port of `org.apache.lucene.util.Sorter`. Implementors provide
/// [`PivotOps::compare`], [`PivotOps::swap`], [`PivotOps::set_pivot`],
/// [`PivotOps::compare_pivot`] and [`Sorter::sort`]; everything else is a
/// faithful port of Lucene's shared helpers.
pub trait Sorter: PivotOps {
    /// Sorts the slice which starts at `from` (inclusive) and ends at `to`
    /// (exclusive).
    fn sort(&mut self, from: usize, to: usize);

    /// Validates that `to >= from`.
    ///
    /// # Panics
    ///
    /// Panics when `to < from`, which is Lucene's `IllegalArgumentException`:
    /// it signals a caller bug, not a property of the data.
    fn check_range(&self, from: usize, to: usize) {
        assert!(
            to >= from,
            "'to' must be >= 'from', got from={from} and to={to}"
        );
    }

    /// Merges the two sorted runs `[from, mid)` and `[mid, to)` in place.
    ///
    /// Equivalent to `Sorter.mergeInPlace`.
    fn merge_in_place(&mut self, from: usize, mid: usize, to: usize) {
        let mut from = from;
        let mut to = to;
        if from == mid || mid == to || self.compare(mid - 1, mid) <= 0 {
            return;
        } else if to - from == 2 {
            self.swap(mid - 1, mid);
            return;
        }
        while self.compare(from, mid) <= 0 {
            from += 1;
        }
        while self.compare(mid - 1, to - 1) <= 0 {
            to -= 1;
        }
        let first_cut;
        let second_cut;
        let len11;
        let len22;
        if mid - from > to - mid {
            len11 = (mid - from) >> 1;
            first_cut = from + len11;
            second_cut = self.lower(mid, to, first_cut);
            len22 = second_cut - mid;
        } else {
            len22 = (to - mid) >> 1;
            second_cut = mid + len22;
            first_cut = self.upper(from, mid, second_cut);
            len11 = first_cut - from;
        }
        let _ = len11;
        self.rotate(first_cut, mid, second_cut);
        let new_mid = first_cut + len22;
        self.merge_in_place(from, first_cut, new_mid);
        self.merge_in_place(new_mid, second_cut, to);
    }

    /// Returns the first index in `[from, to)` whose element is not less than
    /// the element at `val`. Equivalent to `Sorter.lower`.
    fn lower(&mut self, from: usize, to: usize, val: usize) -> usize {
        let mut from = from;
        let mut len = to - from;
        while len > 0 {
            let half = len >> 1;
            let mid = from + half;
            if self.compare(mid, val) < 0 {
                from = mid + 1;
                len = len - half - 1;
            } else {
                len = half;
            }
        }
        from
    }

    /// Returns the first index in `[from, to)` whose element is greater than
    /// the element at `val`. Equivalent to `Sorter.upper`.
    fn upper(&mut self, from: usize, to: usize, val: usize) -> usize {
        let mut from = from;
        let mut len = to - from;
        while len > 0 {
            let half = len >> 1;
            let mid = from + half;
            if self.compare(val, mid) < 0 {
                len = half;
            } else {
                from = mid + 1;
                len = len - half - 1;
            }
        }
        from
    }

    /// Faster than [`Sorter::lower`] when `val` sits at the end of the range.
    ///
    /// Equivalent to `Sorter.lower2`.
    fn lower2(&mut self, from: usize, to: usize, val: usize) -> usize {
        let mut f = to as isize - 1;
        let mut t = to as isize;
        while f > from as isize {
            if self.compare(f as usize, val) < 0 {
                return self.lower(f as usize, t as usize, val);
            }
            let delta = t - f;
            t = f;
            f -= delta << 1;
        }
        self.lower(from, t as usize, val)
    }

    /// Faster than [`Sorter::upper`] when `val` sits at the beginning of the
    /// range. Equivalent to `Sorter.upper2`.
    fn upper2(&mut self, from: usize, to: usize, val: usize) -> usize {
        let mut f = from;
        let mut t = f + 1;
        while t < to {
            if self.compare(t, val) > 0 {
                return self.upper(f, t, val);
            }
            let delta = t - f;
            f = t;
            t += delta << 1;
        }
        self.upper(f, to, val)
    }

    /// Reverses `[from, to)`. Equivalent to `Sorter.reverse`.
    fn reverse(&mut self, from: usize, to: usize) {
        if to == 0 {
            return;
        }
        let mut f = from;
        let mut t = to - 1;
        while f < t {
            self.swap(f, t);
            f += 1;
            t -= 1;
        }
    }

    /// Rotates `[lo, hi)` so that `mid` becomes the first element.
    ///
    /// Equivalent to `Sorter.rotate`.
    fn rotate(&mut self, lo: usize, mid: usize, hi: usize) {
        debug_assert!(lo <= mid && mid <= hi);
        if lo == mid || mid == hi {
            return;
        }
        self.do_rotate(lo, mid, hi);
    }

    /// The rotation strategy, overridable exactly as `Sorter.doRotate` is.
    fn do_rotate(&mut self, lo: usize, mid: usize, hi: usize) {
        if mid - lo == hi - mid {
            // Happens rarely but saves n/2 swaps.
            let mut lo = lo;
            let mut mid = mid;
            while mid < hi {
                self.swap(lo, mid);
                lo += 1;
                mid += 1;
            }
        } else {
            self.reverse(lo, mid);
            self.reverse(mid, hi);
            self.reverse(lo, hi);
        }
    }

    /// Binary sort of `[from, to)`. Equivalent to `Sorter.binarySort(int,int)`.
    fn binary_sort(&mut self, from: usize, to: usize) {
        self.binary_sort_from(from, to, from + 1);
    }

    /// Binary sort of `[from, to)` knowing that `[from, i)` is already sorted.
    ///
    /// Equivalent to `Sorter.binarySort(int,int,int)`.
    fn binary_sort_from(&mut self, from: usize, to: usize, i: usize) {
        let mut i = i;
        while i < to {
            self.set_pivot(i);
            let mut l = from as isize;
            let mut h = i as isize - 1;
            while l <= h {
                let mid = ((l + h) as usize) >> 1;
                let cmp = self.compare_pivot(mid);
                if cmp < 0 {
                    h = mid as isize - 1;
                } else {
                    l = mid as isize + 1;
                }
            }
            let l = l as usize;
            let mut j = i;
            while j > l {
                self.swap(j - 1, j);
                j -= 1;
            }
            i += 1;
        }
    }

    /// Insertion sort of `[from, to)`. Equivalent to `Sorter.insertionSort`.
    fn insertion_sort(&mut self, from: usize, to: usize) {
        let mut i = from + 1;
        while i < to {
            let mut current = i;
            i += 1;
            loop {
                let previous = current - 1;
                if self.compare(previous, current) <= 0 {
                    break;
                }
                self.swap(previous, current);
                if previous == from {
                    break;
                }
                current = previous;
            }
        }
    }

    /// Heap sort of `[from, to)`. Equivalent to `Sorter.heapSort`.
    fn heap_sort(&mut self, from: usize, to: usize) {
        if to - from <= 1 {
            return;
        }
        self.heapify(from, to);
        let mut end = to - 1;
        while end > from {
            self.swap(from, end);
            self.sift_down(from, from, end);
            end -= 1;
        }
    }

    /// Builds the heap over `[from, to)`. Equivalent to `Sorter.heapify`.
    fn heapify(&mut self, from: usize, to: usize) {
        let mut i = heap_parent(from, to - 1);
        loop {
            self.sift_down(i, from, to);
            if i == from {
                break;
            }
            i -= 1;
        }
    }

    /// Sifts the element at `i` down the heap. Equivalent to `Sorter.siftDown`.
    fn sift_down(&mut self, i: usize, from: usize, to: usize) {
        let mut i = i;
        loop {
            let left_child = heap_child(from, i);
            if left_child >= to {
                break;
            }
            let right_child = left_child + 1;
            if self.compare(i, left_child) < 0 {
                if right_child < to && self.compare(left_child, right_child) < 0 {
                    self.swap(i, right_child);
                    i = right_child;
                } else {
                    self.swap(i, left_child);
                    i = left_child;
                }
            } else if right_child < to && self.compare(i, right_child) < 0 {
                self.swap(i, right_child);
                i = right_child;
            } else {
                break;
            }
        }
    }
}

/// Index of the heap parent of `i` in a heap rooted at `from`.
///
/// Equivalent to `Sorter.heapParent`.
pub fn heap_parent(from: usize, i: usize) -> usize {
    ((i - 1 - from) >> 1) + from
}

/// Index of the left heap child of `i` in a heap rooted at `from`.
///
/// Equivalent to `Sorter.heapChild`.
pub fn heap_child(from: usize, i: usize) -> usize {
    ((i - from) << 1) + 1 + from
}

// ---------------------------------------------------------------------------
// InPlaceMergeSorter
// ---------------------------------------------------------------------------

/// A stable merge sort that merges in place, allocating nothing.
///
/// Port of `org.apache.lucene.util.InPlaceMergeSorter`. Implement [`Sorter`]
/// and delegate [`Sorter::sort`] to [`InPlaceMergeSorter::in_place_merge_sort`].
pub trait InPlaceMergeSorter: Sorter {
    /// Entry point: equivalent to `InPlaceMergeSorter.sort(int,int)`.
    fn in_place_merge_sort(&mut self, from: usize, to: usize) {
        self.check_range(from, to);
        self.merge_sort(from, to);
    }

    /// Equivalent to `InPlaceMergeSorter.mergeSort`.
    fn merge_sort(&mut self, from: usize, to: usize) {
        if to - from < BINARY_SORT_THRESHOLD {
            self.binary_sort(from, to);
        } else {
            let mid = (from + to) >> 1;
            self.merge_sort(from, mid);
            self.merge_sort(mid, to);
            self.merge_in_place(from, mid, to);
        }
    }
}

impl<T: Sorter + ?Sized> InPlaceMergeSorter for T {}

// ---------------------------------------------------------------------------
// TimSorter
// ---------------------------------------------------------------------------

/// Minimum run length considered by TimSort. `TimSorter.MINRUN`.
pub const TIM_MINRUN: usize = 32;
/// Length below which TimSort degenerates to a single binary sort.
/// `TimSorter.THRESHOLD`.
pub const TIM_THRESHOLD: usize = 64;
/// Maximum number of pending runs. `TimSorter.STACKSIZE`.
pub const TIM_STACKSIZE: usize = 49;
/// Number of consecutive wins before galloping starts. `TimSorter.MIN_GALLOP`.
pub const TIM_MIN_GALLOP: usize = 7;

/// Minimum run length for an array of length `length`.
///
/// Equivalent to `TimSorter.minRun`.
pub fn tim_min_run(length: usize) -> usize {
    debug_assert!(length >= TIM_MINRUN);
    let mut n = length;
    let mut r = 0usize;
    while n >= 64 {
        r |= n & 1;
        n >>= 1;
    }
    let min_run = n + r;
    debug_assert!((TIM_MINRUN..=TIM_THRESHOLD).contains(&min_run));
    min_run
}

/// The mutable state Java keeps in `TimSorter`'s fields.
///
/// Rust traits cannot hold state, so implementors of [`TimSorter`] own one of
/// these and expose it through [`TimSorter::tim_state`].
#[derive(Debug, Clone)]
pub struct TimSorterState {
    /// Maximum number of slots of temporary storage available for merges.
    pub max_temp_slots: usize,
    /// Minimum run length for the current `sort` call.
    pub min_run: usize,
    /// Exclusive upper bound of the current `sort` call.
    pub to: usize,
    /// Number of pending runs.
    pub stack_size: usize,
    /// Exclusive end offset of every pending run.
    pub run_ends: Vec<usize>,
}

impl TimSorterState {
    /// Creates the state for a sorter allowed `max_temp_slots` slots of
    /// temporary storage. Equivalent to `TimSorter(int)`.
    pub fn new(max_temp_slots: usize) -> Self {
        Self {
            max_temp_slots,
            min_run: 0,
            to: 0,
            stack_size: 0,
            run_ends: vec![0; 1 + TIM_STACKSIZE],
        }
    }
}

/// TimSort, a stable sort that excels on partially sorted input.
///
/// Port of `org.apache.lucene.util.TimSorter`. An implementor must:
///
/// * implement [`Sorter`], forwarding [`Sorter::sort`] to
///   [`TimSorter::tim_sort`] and [`Sorter::do_rotate`] to
///   [`TimSorter::tim_do_rotate`] (Java achieves the latter by overriding
///   `doRotate`, which a Rust subtrait cannot do);
/// * own a [`TimSorterState`] and return it from [`TimSorter::tim_state`].
pub trait TimSorter: Sorter {
    /// Returns the mutable TimSort bookkeeping owned by this sorter.
    fn tim_state(&mut self) -> &mut TimSorterState;

    /// Copies the value in slot `src` into slot `dest`.
    fn copy(&mut self, src: usize, dest: usize);

    /// Saves `[i, i + len)` into the temporary storage.
    fn save(&mut self, i: usize, len: usize);

    /// Restores element `i` of the temporary storage into slot `j`.
    fn restore(&mut self, i: usize, j: usize);

    /// Compares element `i` of the temporary storage with element `j` of the
    /// slice being sorted, like [`PivotOps::compare`].
    fn compare_saved(&mut self, i: usize, j: usize) -> i32;

    /// Maximum number of temporary slots. Equivalent to `TimSorter.maxTempSlots`.
    fn max_temp_slots(&mut self) -> usize {
        self.tim_state().max_temp_slots
    }

    /// Length of the `i`-th pending run. Equivalent to `TimSorter.runLen`.
    fn run_len(&mut self, i: usize) -> usize {
        let s = self.tim_state();
        let off = s.stack_size - i;
        s.run_ends[off] - s.run_ends[off - 1]
    }

    /// Start offset of the `i`-th pending run. Equivalent to `TimSorter.runBase`.
    fn run_base(&mut self, i: usize) -> usize {
        let s = self.tim_state();
        s.run_ends[s.stack_size - i - 1]
    }

    /// End offset of the `i`-th pending run. Equivalent to `TimSorter.runEnd`.
    fn run_end(&mut self, i: usize) -> usize {
        let s = self.tim_state();
        s.run_ends[s.stack_size - i]
    }

    /// Sets the end offset of the `i`-th pending run.
    /// Equivalent to `TimSorter.setRunEnd`.
    fn set_run_end(&mut self, i: usize, run_end: usize) {
        let s = self.tim_state();
        let idx = s.stack_size - i;
        s.run_ends[idx] = run_end;
    }

    /// Pushes a run of length `len`. Equivalent to `TimSorter.pushRunLen`.
    fn push_run_len(&mut self, len: usize) {
        let s = self.tim_state();
        s.run_ends[s.stack_size + 1] = s.run_ends[s.stack_size] + len;
        s.stack_size += 1;
    }

    /// Computes, sorts and returns the length of the next run.
    ///
    /// Equivalent to `TimSorter.nextRun`.
    fn next_run(&mut self) -> usize {
        let run_base = self.run_end(0);
        let to = self.tim_state().to;
        debug_assert!(run_base < to);
        if run_base == to - 1 {
            return 1;
        }
        let mut o = run_base + 2;
        if self.compare(run_base, run_base + 1) > 0 {
            // The run must be strictly descending.
            while o < to && self.compare(o - 1, o) > 0 {
                o += 1;
            }
            self.reverse(run_base, o);
        } else {
            // The run must be non-descending.
            while o < to && self.compare(o - 1, o) <= 0 {
                o += 1;
            }
        }
        let min_run = self.tim_state().min_run;
        let run_hi = o.max(to.min(run_base + min_run));
        self.binary_sort_from(run_base, run_hi, o);
        run_hi - run_base
    }

    /// Restores TimSort's stack invariants. Equivalent to
    /// `TimSorter.ensureInvariants`.
    fn ensure_invariants(&mut self) {
        while self.tim_state().stack_size > 1 {
            let run_len0 = self.run_len(0);
            let run_len1 = self.run_len(1);

            if self.tim_state().stack_size > 2 {
                let run_len2 = self.run_len(2);
                if run_len2 <= run_len1 + run_len0 {
                    if run_len2 < run_len0 {
                        self.merge_at(1);
                    } else {
                        self.merge_at(0);
                    }
                    continue;
                }
            }

            if run_len1 <= run_len0 {
                self.merge_at(0);
                continue;
            }

            break;
        }
    }

    /// Merges every pending run. Equivalent to `TimSorter.exhaustStack`.
    fn exhaust_stack(&mut self) {
        while self.tim_state().stack_size > 1 {
            self.merge_at(0);
        }
    }

    /// Resets the bookkeeping for a new `[from, to)` sort.
    ///
    /// Equivalent to `TimSorter.reset`.
    fn reset(&mut self, from: usize, to: usize) {
        let length = to - from;
        let min_run = if length <= TIM_THRESHOLD {
            length
        } else {
            tim_min_run(length)
        };
        let s = self.tim_state();
        s.stack_size = 0;
        s.run_ends.iter_mut().for_each(|v| *v = 0);
        s.run_ends[0] = from;
        s.to = to;
        s.min_run = min_run;
    }

    /// Merges the run at `n` with the one below it.
    ///
    /// Equivalent to `TimSorter.mergeAt`.
    fn merge_at(&mut self, n: usize) {
        debug_assert!(self.tim_state().stack_size >= 2);
        let lo = self.run_base(n + 1);
        let mid = self.run_base(n);
        let hi = self.run_end(n);
        self.merge(lo, mid, hi);
        let mut j = n + 1;
        while j > 0 {
            let e = self.run_end(j - 1);
            self.set_run_end(j, e);
            j -= 1;
        }
        self.tim_state().stack_size -= 1;
    }

    /// Merges the two adjacent sorted runs `[lo, mid)` and `[mid, hi)`.
    ///
    /// Equivalent to `TimSorter.merge`.
    fn merge(&mut self, lo: usize, mid: usize, hi: usize) {
        if self.compare(mid - 1, mid) <= 0 {
            return;
        }
        let lo = self.upper2(lo, mid, mid);
        let hi = self.lower2(mid, hi, mid - 1);

        let max_temp_slots = self.max_temp_slots();
        if hi - mid <= mid - lo && hi - mid <= max_temp_slots {
            self.merge_hi(lo, mid, hi);
        } else if mid - lo <= max_temp_slots {
            self.merge_lo(lo, mid, hi);
        } else {
            self.merge_in_place(lo, mid, hi);
        }
    }

    /// Entry point: equivalent to `TimSorter.sort(int,int)`.
    fn tim_sort(&mut self, from: usize, to: usize) {
        self.check_range(from, to);
        if to - from <= 1 {
            return;
        }
        self.reset(from, to);
        loop {
            self.ensure_invariants();
            let len = self.next_run();
            self.push_run_len(len);
            if self.run_end(0) >= to {
                break;
            }
        }
        self.exhaust_stack();
        debug_assert_eq!(self.run_end(0), to);
    }

    /// The rotation strategy TimSort uses, which takes advantage of the
    /// temporary storage. Equivalent to `TimSorter.doRotate`.
    fn tim_do_rotate(&mut self, lo: usize, mid: usize, hi: usize) {
        let len1 = mid - lo;
        let len2 = hi - mid;
        let max_temp_slots = self.max_temp_slots();
        if len1 == len2 {
            let mut lo = lo;
            let mut mid = mid;
            while mid < hi {
                self.swap(lo, mid);
                lo += 1;
                mid += 1;
            }
        } else if len2 < len1 && len2 <= max_temp_slots {
            self.save(mid, len2);
            let mut i = lo + len1 - 1;
            let mut j = hi - 1;
            loop {
                self.copy(i, j);
                if i == lo {
                    break;
                }
                i -= 1;
                j -= 1;
            }
            for i in 0..len2 {
                self.restore(i, lo + i);
            }
        } else if len1 <= max_temp_slots {
            self.save(lo, len1);
            for (j, i) in (lo..).zip(mid..hi) {
                self.copy(i, j);
            }
            let mut i = 0;
            let mut j = lo + len2;
            while j < hi {
                self.restore(i, j);
                i += 1;
                j += 1;
            }
        } else {
            self.reverse(lo, mid);
            self.reverse(mid, hi);
            self.reverse(lo, hi);
        }
    }

    /// Merge that buffers the lower run. Equivalent to `TimSorter.mergeLo`.
    fn merge_lo(&mut self, lo: usize, mid: usize, hi: usize) {
        debug_assert!(self.compare(lo, mid) > 0);
        let len1 = mid - lo;
        self.save(lo, len1);
        self.copy(mid, lo);
        let mut i = 0usize;
        let mut j = mid + 1;
        let mut dest = lo + 1;
        'outer: loop {
            let mut count = 0usize;
            while count < TIM_MIN_GALLOP {
                if i >= len1 || j >= hi {
                    break 'outer;
                } else if self.compare_saved(i, j) <= 0 {
                    self.restore(i, dest);
                    i += 1;
                    dest += 1;
                    count = 0;
                } else {
                    self.copy(j, dest);
                    j += 1;
                    dest += 1;
                    count += 1;
                }
            }
            // Galloping...
            let next = self.lower_saved3(j, hi, i);
            while j < next {
                self.copy(j, dest);
                j += 1;
                dest += 1;
            }
            self.restore(i, dest);
            i += 1;
            dest += 1;
        }
        while i < len1 {
            self.restore(i, dest);
            i += 1;
            dest += 1;
        }
        debug_assert_eq!(j, dest);
    }

    /// Merge that buffers the upper run. Equivalent to `TimSorter.mergeHi`.
    fn merge_hi(&mut self, lo: usize, mid: usize, hi: usize) {
        debug_assert!(self.compare(mid - 1, hi - 1) > 0);
        let len2 = hi - mid;
        self.save(mid, len2);
        self.copy(mid - 1, hi - 1);
        // Java runs these three cursors down through -1, so they are signed here.
        let mut i = mid as isize - 2;
        let mut j = len2 as isize - 1;
        let mut dest = hi as isize - 2;
        'outer: loop {
            let mut count = 0usize;
            while count < TIM_MIN_GALLOP {
                if i < lo as isize || j < 0 {
                    break 'outer;
                } else if self.compare_saved(j as usize, i as usize) >= 0 {
                    self.restore(j as usize, dest as usize);
                    j -= 1;
                    dest -= 1;
                    count = 0;
                } else {
                    self.copy(i as usize, dest as usize);
                    i -= 1;
                    dest -= 1;
                    count += 1;
                }
            }
            // Galloping...
            let next = self.upper_saved3(lo, (i + 1) as usize, j as usize) as isize;
            while i >= next {
                self.copy(i as usize, dest as usize);
                i -= 1;
                dest -= 1;
            }
            self.restore(j as usize, dest as usize);
            j -= 1;
            dest -= 1;
        }
        while j >= 0 {
            self.restore(j as usize, dest as usize);
            j -= 1;
            dest -= 1;
        }
        debug_assert_eq!(i, dest);
    }

    /// Equivalent to `TimSorter.lowerSaved`.
    fn lower_saved(&mut self, from: usize, to: usize, val: usize) -> usize {
        let mut from = from;
        let mut len = to - from;
        while len > 0 {
            let half = len >> 1;
            let mid = from + half;
            if self.compare_saved(val, mid) > 0 {
                from = mid + 1;
                len = len - half - 1;
            } else {
                len = half;
            }
        }
        from
    }

    /// Equivalent to `TimSorter.upperSaved`.
    fn upper_saved(&mut self, from: usize, to: usize, val: usize) -> usize {
        let mut from = from;
        let mut len = to - from;
        while len > 0 {
            let half = len >> 1;
            let mid = from + half;
            if self.compare_saved(val, mid) < 0 {
                len = half;
            } else {
                from = mid + 1;
                len = len - half - 1;
            }
        }
        from
    }

    /// Equivalent to `TimSorter.lowerSaved3`.
    fn lower_saved3(&mut self, from: usize, to: usize, val: usize) -> usize {
        let mut f = from;
        let mut t = f + 1;
        while t < to {
            if self.compare_saved(val, t) <= 0 {
                return self.lower_saved(f, t, val);
            }
            let delta = t - f;
            f = t;
            t += delta << 1;
        }
        self.lower_saved(f, to, val)
    }

    /// Equivalent to `TimSorter.upperSaved3`.
    fn upper_saved3(&mut self, from: usize, to: usize, val: usize) -> usize {
        let mut f = to as isize - 1;
        let mut t = to as isize;
        while f > from as isize {
            if self.compare_saved(val, f as usize) >= 0 {
                return self.upper_saved(f as usize, t as usize, val);
            }
            let delta = t - f;
            t = f;
            f -= delta << 1;
        }
        self.upper_saved(from, t as usize, val)
    }
}

// ---------------------------------------------------------------------------
// Array sorters
// ---------------------------------------------------------------------------

/// An [`InPlaceMergeSorter`] over a mutable slice.
///
/// Port of `org.apache.lucene.util.ArrayInPlaceMergeSorter`.
pub struct ArrayInPlaceMergeSorter<'a, T, C>
where
    C: FnMut(&T, &T) -> Ordering,
{
    arr: &'a mut [T],
    comparator: C,
    pivot_index: usize,
}

impl<'a, T, C> ArrayInPlaceMergeSorter<'a, T, C>
where
    C: FnMut(&T, &T) -> Ordering,
{
    /// Creates a sorter over `arr` ordered by `comparator`.
    pub fn new(arr: &'a mut [T], comparator: C) -> Self {
        Self {
            arr,
            comparator,
            pivot_index: 0,
        }
    }
}

impl<T, C> PivotOps for ArrayInPlaceMergeSorter<'_, T, C>
where
    C: FnMut(&T, &T) -> Ordering,
{
    fn swap(&mut self, i: usize, j: usize) {
        self.arr.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot_index = i;
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.compare(self.pivot_index, j)
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        let Self {
            arr, comparator, ..
        } = self;
        ordering_to_int(comparator(&arr[i], &arr[j]))
    }
}

impl<T, C> Sorter for ArrayInPlaceMergeSorter<'_, T, C>
where
    C: FnMut(&T, &T) -> Ordering,
{
    fn sort(&mut self, from: usize, to: usize) {
        self.in_place_merge_sort(from, to);
    }
}

/// An intro sorter over a mutable slice.
///
/// Port of `org.apache.lucene.util.ArrayIntroSorter`.
///
/// **Divergence from Lucene 10.5.0.** Java stores the pivot as a reference to
/// the array element (`T pivot`). Rust cannot hold a borrow into the slice
/// while the slice is being permuted, so the pivot is a clone; hence the
/// `T: Clone` bound, which Java does not need.
pub struct ArrayIntroSorter<'a, T, C>
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    arr: &'a mut [T],
    comparator: C,
    pivot: Option<T>,
}

impl<'a, T, C> ArrayIntroSorter<'a, T, C>
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    /// Creates a sorter over `arr` ordered by `comparator`.
    pub fn new(arr: &'a mut [T], comparator: C) -> Self {
        Self {
            arr,
            comparator,
            pivot: None,
        }
    }
}

impl<T, C> PivotOps for ArrayIntroSorter<'_, T, C>
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    fn swap(&mut self, i: usize, j: usize) {
        self.arr.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot = Some(self.arr[i].clone());
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        let Self {
            arr,
            comparator,
            pivot,
        } = self;
        let pivot = pivot
            .as_ref()
            .expect("INVARIANT: set_pivot precedes compare_pivot");
        ordering_to_int(comparator(pivot, &arr[j]))
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        let Self {
            arr, comparator, ..
        } = self;
        ordering_to_int(comparator(&arr[i], &arr[j]))
    }
}

impl<T, C> Sorter for ArrayIntroSorter<'_, T, C>
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    fn sort(&mut self, from: usize, to: usize) {
        intro_sort(self, from, to);
    }
}

/// A [`TimSorter`] over a mutable slice.
///
/// Port of `org.apache.lucene.util.ArrayTimSorter`.
///
/// **Divergence from Lucene 10.5.0.** Java's temporary storage is an
/// `Object[]` holding references; this port holds `Option<T>` clones, mirroring
/// the null-initialised Java array while satisfying Rust's ownership rules.
/// Hence the `T: Clone` bound, which Java does not need.
pub struct ArrayTimSorter<'a, T, C>
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    arr: &'a mut [T],
    comparator: C,
    tmp: Vec<Option<T>>,
    pivot_index: usize,
    state: TimSorterState,
}

impl<'a, T, C> ArrayTimSorter<'a, T, C>
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    /// Creates a sorter over `arr` with at most `max_temp_slots` slots of
    /// temporary storage.
    pub fn new(arr: &'a mut [T], comparator: C, max_temp_slots: usize) -> Self {
        Self {
            arr,
            comparator,
            tmp: vec![None; max_temp_slots],
            pivot_index: 0,
            state: TimSorterState::new(max_temp_slots),
        }
    }
}

impl<T, C> PivotOps for ArrayTimSorter<'_, T, C>
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    fn swap(&mut self, i: usize, j: usize) {
        self.arr.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot_index = i;
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.compare(self.pivot_index, j)
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        let Self {
            arr, comparator, ..
        } = self;
        ordering_to_int(comparator(&arr[i], &arr[j]))
    }
}

impl<T, C> Sorter for ArrayTimSorter<'_, T, C>
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    fn sort(&mut self, from: usize, to: usize) {
        self.tim_sort(from, to);
    }

    fn do_rotate(&mut self, lo: usize, mid: usize, hi: usize) {
        self.tim_do_rotate(lo, mid, hi);
    }
}

impl<T, C> TimSorter for ArrayTimSorter<'_, T, C>
where
    T: Clone,
    C: FnMut(&T, &T) -> Ordering,
{
    fn tim_state(&mut self) -> &mut TimSorterState {
        &mut self.state
    }

    fn copy(&mut self, src: usize, dest: usize) {
        self.arr[dest] = self.arr[src].clone();
    }

    fn save(&mut self, start: usize, len: usize) {
        for i in 0..len {
            self.tmp[i] = Some(self.arr[start + i].clone());
        }
    }

    fn restore(&mut self, src: usize, dest: usize) {
        self.arr[dest] = self.tmp[src]
            .clone()
            .expect("INVARIANT: restore only reads slots written by save");
    }

    fn compare_saved(&mut self, i: usize, j: usize) -> i32 {
        let Self {
            arr,
            comparator,
            tmp,
            ..
        } = self;
        let saved = tmp[i]
            .as_ref()
            .expect("INVARIANT: compare_saved only reads slots written by save");
        ordering_to_int(comparator(saved, &arr[j]))
    }
}

// ---------------------------------------------------------------------------
// MSBRadixSorter
// ---------------------------------------------------------------------------

/// Recursion depth after which the MSB radix sort falls back to intro sort.
/// `MSBRadixSorter.LEVEL_THRESHOLD`.
pub const MSB_LEVEL_THRESHOLD: usize = 8;
/// 256 byte values plus one slot meaning "the string ended here".
/// `MSBRadixSorter.HISTOGRAM_SIZE`.
pub const MSB_HISTOGRAM_SIZE: usize = 257;
/// Buckets at or below this size are sorted with the fallback sorter.
/// `MSBRadixSorter.LENGTH_THRESHOLD`.
pub const MSB_LENGTH_THRESHOLD: usize = 100;

/// The operations [`MSBRadixSorter`] performs on the range it sorts.
///
/// Port of the abstract methods of `org.apache.lucene.util.MSBRadixSorter`.
pub trait MSBRadixSorterOps {
    /// Returns the `k`-th byte of the entry at index `i` as an unsigned value,
    /// or `-1` when the entry is at most `k` bytes long.
    fn byte_at(&self, i: usize, k: usize) -> i32;

    /// Exchanges the entries at `i` and `j`.
    fn swap(&mut self, i: usize, j: usize);

    /// Returns the histogram bucket of the `k`-th byte of entry `i`.
    ///
    /// Equivalent to `MSBRadixSorter.getBucket`.
    fn get_bucket(&self, i: usize, k: usize) -> usize {
        (self.byte_at(i, k) + 1) as usize
    }

    /// Builds the bucket histogram of `[from, to)`.
    ///
    /// Equivalent to `MSBRadixSorter.buildHistogram`.
    fn build_histogram(
        &self,
        prefix_common_bucket: usize,
        prefix_common_len: i32,
        from: usize,
        to: usize,
        k: usize,
        histogram: &mut [i32],
    ) {
        histogram[prefix_common_bucket] = prefix_common_len;
        for i in from..to {
            histogram[self.get_bucket(i, k)] += 1;
        }
    }

    /// Reorders `[from, to)` so that every entry lands in its bucket.
    ///
    /// When this returns, `start_offsets` and `end_offsets` are equal. The
    /// default is Lucene's in-place Dutch-flag reordering, which is **not**
    /// stable; [`stable_reorder`] is the stable replacement.
    fn reorder(
        &mut self,
        from: usize,
        to: usize,
        start_offsets: &mut [i32],
        end_offsets: &[i32],
        k: usize,
    ) {
        let _ = to;
        for i in 0..MSB_HISTOGRAM_SIZE {
            let limit = end_offsets[i];
            loop {
                let h1 = start_offsets[i];
                if h1 >= limit {
                    break;
                }
                let b = self.get_bucket(from + h1 as usize, k);
                let h2 = start_offsets[b];
                start_offsets[b] += 1;
                self.swap(from + h1 as usize, from + h2 as usize);
            }
        }
    }

    /// Whether the range should be handed to the fallback sorter.
    ///
    /// Equivalent to `MSBRadixSorter.shouldFallback`.
    fn should_fallback(&self, from: usize, to: usize, l: usize) -> bool {
        to - from <= MSB_LENGTH_THRESHOLD || l >= MSB_LEVEL_THRESHOLD
    }

    /// Sorts `[from, to)` knowing that the first `k` bytes of every entry are
    /// equal.
    ///
    /// Equivalent to `MSBRadixSorter.getFallbackSorter(k).sort(from, to)`; the
    /// default is Lucene's intro sort over [`MSBRadixSorterOps::byte_at`].
    fn fallback_sort(&mut self, from: usize, to: usize, k: usize, max_length: usize) {
        let mut fallback = MsbFallbackSorter {
            ops: self,
            k,
            max_length,
            pivot: Vec::new(),
        };
        intro_sort(&mut fallback, from, to);
    }
}

/// Lucene's anonymous `IntroSorter` returned by
/// `MSBRadixSorter.getFallbackSorter(int)`.
struct MsbFallbackSorter<'a, O: MSBRadixSorterOps + ?Sized> {
    ops: &'a mut O,
    k: usize,
    max_length: usize,
    pivot: Vec<u8>,
}

impl<O: MSBRadixSorterOps + ?Sized> PivotOps for MsbFallbackSorter<'_, O> {
    fn swap(&mut self, i: usize, j: usize) {
        self.ops.swap(i, j);
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        for o in self.k..self.max_length {
            let b1 = self.ops.byte_at(i, o);
            let b2 = self.ops.byte_at(j, o);
            if b1 != b2 {
                return b1 - b2;
            } else if b1 == -1 {
                break;
            }
        }
        0
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot.clear();
        for o in self.k..self.max_length {
            let b = self.ops.byte_at(i, o);
            if b == -1 {
                break;
            }
            self.pivot.push(b as u8);
        }
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        for o in 0..self.pivot.len() {
            let b1 = self.pivot[o] as i32;
            let b2 = self.ops.byte_at(j, self.k + o);
            if b1 != b2 {
                return b1 - b2;
            }
        }
        if self.k + self.pivot.len() == self.max_length {
            return 0;
        }
        -1 - self.ops.byte_at(j, self.k + self.pivot.len())
    }
}

/// Radix sorter for variable-length strings, sorting on the most significant
/// byte first. **Not** a stable sort.
///
/// Port of `org.apache.lucene.util.MSBRadixSorter`. The per-instance scratch
/// (one histogram per recursion level, the end-offset array and the
/// common-prefix buffer) lives in this struct exactly as it does in the Java
/// class; the abstract methods live in [`MSBRadixSorterOps`].
pub struct MSBRadixSorter {
    max_length: usize,
    histograms: Vec<Option<Vec<i32>>>,
    end_offsets: Vec<i32>,
    common_prefix: Vec<i32>,
}

impl MSBRadixSorter {
    /// Creates a sorter for keys of at most `max_length` bytes.
    ///
    /// Pass `i32::MAX as usize` when the maximum length is unknown, exactly as
    /// Java passes `Integer.MAX_VALUE`.
    pub fn new(max_length: usize) -> Self {
        Self {
            max_length,
            histograms: (0..MSB_LEVEL_THRESHOLD).map(|_| None).collect(),
            end_offsets: vec![0; MSB_HISTOGRAM_SIZE],
            common_prefix: vec![0; max_length.min(24)],
        }
    }

    /// Returns the maximum key length this sorter was built for.
    pub fn max_length(&self) -> usize {
        self.max_length
    }

    /// Sorts `[from, to)`.
    ///
    /// # Panics
    ///
    /// Panics when `to < from`, which is Lucene's `IllegalArgumentException`.
    pub fn sort<O: MSBRadixSorterOps + ?Sized>(&mut self, ops: &mut O, from: usize, to: usize) {
        assert!(
            to >= from,
            "'to' must be >= 'from', got from={from} and to={to}"
        );
        self.sort_at(ops, from, to, 0, 0);
    }

    /// Equivalent to `MSBRadixSorter.sort(int,int,int,int)`.
    fn sort_at<O: MSBRadixSorterOps + ?Sized>(
        &mut self,
        ops: &mut O,
        from: usize,
        to: usize,
        k: usize,
        l: usize,
    ) {
        if ops.should_fallback(from, to, l) {
            ops.fallback_sort(from, to, k, self.max_length);
        } else {
            self.radix_sort(ops, from, to, k, l);
        }
    }

    /// Equivalent to `MSBRadixSorter.radixSort`.
    fn radix_sort<O: MSBRadixSorterOps + ?Sized>(
        &mut self,
        ops: &mut O,
        from: usize,
        to: usize,
        k: usize,
        l: usize,
    ) {
        let mut histogram = match self.histograms[l].take() {
            Some(mut h) => {
                h.iter_mut().for_each(|v| *v = 0);
                h
            }
            None => vec![0i32; MSB_HISTOGRAM_SIZE],
        };

        let common_prefix_length =
            self.compute_common_prefix_length_and_build_histogram(ops, from, to, k, &mut histogram);
        if common_prefix_length > 0 {
            // If there are no more bytes to compare, or if every entry fell into
            // the first bucket (meaning the keys are shorter than `k`), we are
            // done; otherwise recurse past the common prefix.
            let recurse =
                k + common_prefix_length < self.max_length && (histogram[0] as usize) < to - from;
            self.histograms[l] = Some(histogram);
            if recurse {
                self.radix_sort(ops, from, to, k + common_prefix_length, l);
            }
            return;
        }

        // `startOffsets` and `endOffsets` are Lucene's two views of the same
        // counters; after `reorder` they are equal, and Java then reads the
        // bucket bounds off `startOffsets`.
        let mut end_offsets = std::mem::take(&mut self.end_offsets);
        if end_offsets.len() != MSB_HISTOGRAM_SIZE {
            end_offsets = vec![0i32; MSB_HISTOGRAM_SIZE];
        }
        sum_histogram(&mut histogram, &mut end_offsets);
        ops.reorder(from, to, &mut histogram, &end_offsets, k);
        self.end_offsets = end_offsets;

        if k + 1 < self.max_length {
            // Recurse on all but the first bucket: its keys are all equal
            // because every one of their bytes has already been compared.
            let mut prev = histogram[0];
            for &h in histogram.iter().take(MSB_HISTOGRAM_SIZE).skip(1) {
                let bucket_len = h - prev;
                if bucket_len > 1 {
                    self.sort_at(ops, from + prev as usize, from + h as usize, k + 1, l + 1);
                }
                prev = h;
            }
        }

        self.histograms[l] = Some(histogram);
    }

    /// Builds the histogram of `[from, to)` and returns the length of the
    /// common prefix shared by every entry.
    ///
    /// Equivalent to `MSBRadixSorter.computeCommonPrefixLengthAndBuildHistogram`.
    /// Java splits this into three methods to work around a JVM crash
    /// (<https://github.com/apache/lucene/issues/12898>); that split has no
    /// meaning outside the JVM, so this port keeps one method.
    fn compute_common_prefix_length_and_build_histogram<O: MSBRadixSorterOps + ?Sized>(
        &mut self,
        ops: &O,
        from: usize,
        to: usize,
        k: usize,
        histogram: &mut [i32],
    ) -> usize {
        let mut common_prefix_length = self.common_prefix.len().min(self.max_length - k);
        for j in 0..common_prefix_length {
            let b = ops.byte_at(from, k + j);
            self.common_prefix[j] = b;
            if b == -1 {
                common_prefix_length = j + 1;
                break;
            }
        }

        let mut i = from + 1;
        'outer: while i < to {
            // Java shortens `commonPrefixLength` inside this loop and breaks
            // immediately, so the bound is only ever read again on the next
            // outer iteration; an explicit `while` makes that plain.
            let mut j = 0usize;
            while j < common_prefix_length {
                let b = ops.byte_at(i, k + j);
                if b != self.common_prefix[j] {
                    common_prefix_length = j;
                    if common_prefix_length == 0 {
                        // No common prefix at all.
                        break 'outer;
                    }
                    break;
                }
                j += 1;
            }
            i += 1;
        }

        if i < to {
            debug_assert_eq!(common_prefix_length, 0);
            ops.build_histogram(
                (self.common_prefix[0] + 1) as usize,
                (i - from) as i32,
                i,
                to,
                k,
                histogram,
            );
        } else {
            debug_assert!(common_prefix_length > 0);
            histogram[(self.common_prefix[0] + 1) as usize] = (to - from) as i32;
        }

        common_prefix_length
    }
}

/// Turns bucket counts into start offsets, writing the end offsets into
/// `end_offsets`. Equivalent to `MSBRadixSorter.sumHistogram`.
fn sum_histogram(histogram: &mut [i32], end_offsets: &mut [i32]) {
    let mut accum = 0i32;
    for i in 0..MSB_HISTOGRAM_SIZE {
        let count = histogram[i];
        histogram[i] = accum;
        accum += count;
        end_offsets[i] = accum;
    }
}

// ---------------------------------------------------------------------------
// StableMSBRadixSorter
// ---------------------------------------------------------------------------

/// A merge sorter that takes advantage of temporary storage.
///
/// Port of the nested class `StableMSBRadixSorter.MergeSorter`. Implement
/// [`Sorter`] and delegate [`Sorter::sort`] to
/// [`MergeSorter::merge_sorter_sort`].
pub trait MergeSorter: Sorter {
    /// Saves the `i`-th value into the `j`-th position of temporary storage.
    fn save(&mut self, i: usize, j: usize);

    /// Restores values `[i, j)` from temporary storage into the original one.
    fn restore(&mut self, i: usize, j: usize);

    /// Entry point: equivalent to `MergeSorter.sort(int,int)`.
    fn merge_sorter_sort(&mut self, from: usize, to: usize) {
        self.check_range(from, to);
        self.merge_sorter_merge_sort(from, to);
    }

    /// Equivalent to `MergeSorter.mergeSort`.
    fn merge_sorter_merge_sort(&mut self, from: usize, to: usize) {
        if to - from < BINARY_SORT_THRESHOLD {
            self.binary_sort(from, to);
        } else {
            let mid = (from + to) >> 1;
            self.merge_sorter_merge_sort(from, mid);
            self.merge_sorter_merge_sort(mid, to);
            self.merge_sorter_merge(from, to, mid);
        }
    }

    /// Equivalent to `MergeSorter.bulkSave`.
    fn bulk_save(&mut self, from: usize, tmp_from: usize, len: usize) {
        for i in 0..len {
            self.save(from + i, tmp_from + i);
        }
    }

    /// Equivalent to `MergeSorter.merge`.
    fn merge_sorter_merge(&mut self, from: usize, to: usize, mid: usize) {
        debug_assert!(to > mid && mid > from);
        if self.compare(mid - 1, mid) <= 0 {
            // Already sorted.
            return;
        }
        let mut left = from;
        let mut right = mid;
        let mut index = from;
        loop {
            let cmp = self.compare(left, right);
            if cmp <= 0 {
                self.save(left, index);
                left += 1;
                index += 1;
                if left == mid {
                    debug_assert_eq!(index, right);
                    self.bulk_save(right, index, to - right);
                    break;
                }
            } else {
                self.save(right, index);
                right += 1;
                index += 1;
                if right == to {
                    debug_assert_eq!(to - index, mid - left);
                    self.bulk_save(left, index, mid - left);
                    break;
                }
            }
        }
        self.restore(from, to);
    }
}

/// The extra operations a stable MSB radix sort needs on top of
/// [`MSBRadixSorterOps`].
///
/// Port of the abstract `save`/`restore` of
/// `org.apache.lucene.util.StableMSBRadixSorter`.
pub trait StableMSBRadixSorterOps: MSBRadixSorterOps {
    /// Saves the `i`-th value into the `j`-th position of temporary storage.
    fn save(&mut self, i: usize, j: usize);

    /// Restores values `[i, j)` from temporary storage into the original one.
    fn restore(&mut self, i: usize, j: usize);
}

/// Stable replacement for [`MSBRadixSorterOps::reorder`].
///
/// Port of `StableMSBRadixSorter.reorder`. Implementors of
/// [`StableMSBRadixSorterOps`] override `reorder` to call this.
///
/// **Divergence from Lucene 10.5.0.** Java caches the untouched copy of
/// `startOffsets` in a `fixedStartOffsets` field so that repeated sorts reuse
/// one allocation. This port keeps it on the stack: it is a fixed 257-element
/// array, so no allocation happens either way, and the result is identical.
pub fn stable_reorder<O: StableMSBRadixSorterOps + ?Sized>(
    ops: &mut O,
    from: usize,
    to: usize,
    start_offsets: &mut [i32],
    end_offsets: &[i32],
    k: usize,
) {
    let mut fixed_start_offsets = [0i32; MSB_HISTOGRAM_SIZE];
    fixed_start_offsets.copy_from_slice(&start_offsets[..MSB_HISTOGRAM_SIZE]);
    for i in 0..MSB_HISTOGRAM_SIZE {
        let limit = end_offsets[i];
        let mut h1 = fixed_start_offsets[i];
        while h1 < limit {
            let b = ops.get_bucket(from + h1 as usize, k);
            let h2 = start_offsets[b];
            start_offsets[b] += 1;
            ops.save(from + h1 as usize, from + h2 as usize);
            h1 += 1;
        }
    }
    ops.restore(from, to);
}

/// Lucene's `StableMSBRadixSorter.getFallbackSorter(int)`: a [`MergeSorter`]
/// comparing on [`MSBRadixSorterOps::byte_at`].
///
/// Implementors of [`StableMSBRadixSorterOps`] override
/// [`MSBRadixSorterOps::fallback_sort`] to call this.
pub fn stable_fallback_sort<O: StableMSBRadixSorterOps + ?Sized>(
    ops: &mut O,
    from: usize,
    to: usize,
    k: usize,
    max_length: usize,
) {
    let mut sorter = StableFallbackSorter {
        ops,
        k,
        max_length,
        pivot_index: 0,
    };
    sorter.merge_sorter_sort(from, to);
}

struct StableFallbackSorter<'a, O: StableMSBRadixSorterOps + ?Sized> {
    ops: &'a mut O,
    k: usize,
    max_length: usize,
    pivot_index: usize,
}

impl<O: StableMSBRadixSorterOps + ?Sized> PivotOps for StableFallbackSorter<'_, O> {
    fn swap(&mut self, i: usize, j: usize) {
        self.ops.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot_index = i;
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.compare(self.pivot_index, j)
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        for o in self.k..self.max_length {
            let b1 = self.ops.byte_at(i, o);
            let b2 = self.ops.byte_at(j, o);
            if b1 != b2 {
                return b1 - b2;
            } else if b1 == -1 {
                break;
            }
        }
        0
    }
}

impl<O: StableMSBRadixSorterOps + ?Sized> Sorter for StableFallbackSorter<'_, O> {
    fn sort(&mut self, from: usize, to: usize) {
        self.merge_sorter_sort(from, to);
    }
}

impl<O: StableMSBRadixSorterOps + ?Sized> MergeSorter for StableFallbackSorter<'_, O> {
    fn save(&mut self, i: usize, j: usize) {
        self.ops.save(i, j);
    }

    fn restore(&mut self, i: usize, j: usize) {
        self.ops.restore(i, j);
    }
}

/// Stable radix sorter for variable-length strings.
///
/// Port of `org.apache.lucene.util.StableMSBRadixSorter`. Java obtains
/// stability by overriding `reorder` and `getFallbackSorter`; because a Rust
/// subtrait cannot override a supertrait's default method, an implementor of
/// [`StableMSBRadixSorterOps`] forwards [`MSBRadixSorterOps::reorder`] to
/// [`stable_reorder`] and [`MSBRadixSorterOps::fallback_sort`] to
/// [`stable_fallback_sort`].
pub struct StableMSBRadixSorter {
    inner: MSBRadixSorter,
}

impl StableMSBRadixSorter {
    /// Creates a stable sorter for keys of at most `max_length` bytes.
    pub fn new(max_length: usize) -> Self {
        Self {
            inner: MSBRadixSorter::new(max_length),
        }
    }

    /// Returns the maximum key length this sorter was built for.
    pub fn max_length(&self) -> usize {
        self.inner.max_length()
    }

    /// Sorts `[from, to)` stably.
    pub fn sort<O: StableMSBRadixSorterOps>(&mut self, ops: &mut O, from: usize, to: usize) {
        self.inner.sort(ops, from, to);
    }
}

// ---------------------------------------------------------------------------
// LSBRadixSorter
// ---------------------------------------------------------------------------

/// Number of entries below which [`LSBRadixSorter`] uses insertion sort.
/// `LSBRadixSorter.INSERTION_SORT_THRESHOLD`.
const LSB_INSERTION_SORT_THRESHOLD: usize = 30;
/// `LSBRadixSorter.HISTOGRAM_SIZE`.
const LSB_HISTOGRAM_SIZE: usize = 256;

/// An LSB radix sorter for unsigned `i32` values.
///
/// Port of `org.apache.lucene.util.LSBRadixSorter`.
#[derive(Debug)]
pub struct LSBRadixSorter {
    histogram: [i32; LSB_HISTOGRAM_SIZE],
    buffer: Vec<i32>,
}

impl Default for LSBRadixSorter {
    fn default() -> Self {
        Self::new()
    }
}

impl LSBRadixSorter {
    /// Creates a sorter with an empty scratch buffer.
    pub fn new() -> Self {
        Self {
            histogram: [0; LSB_HISTOGRAM_SIZE],
            buffer: Vec::new(),
        }
    }

    /// Sorts `array[0..len]` in place.
    ///
    /// `num_bits` is how many bits are required to store any of the values in
    /// `array[0..len]`; pass `32` when unknown.
    pub fn sort(&mut self, num_bits: u32, array: &mut [i32], len: usize) {
        if len < LSB_INSERTION_SORT_THRESHOLD {
            insertion_sort_ints(array, 0, len);
            return;
        }

        if self.buffer.len() < len {
            // `ArrayUtil.growNoCopy`: the previous content is never read again.
            self.buffer = vec![0i32; ArrayUtil::oversize(len, 4).max(len)];
        }

        // Java swaps the `arr` and `buf` pointers; Rust cannot alias a borrowed
        // slice and an owned buffer under one name, so the same alternation is
        // tracked by a flag and the final copy-back is driven by it. The
        // sequence of passes, and therefore the result, is identical.
        let mut in_buffer = false;
        let mut shift = 0u32;
        while shift < num_bits {
            let moved = {
                let Self { histogram, buffer } = self;
                histogram.iter_mut().for_each(|v| *v = 0);
                if in_buffer {
                    Self::sort_pass(&buffer[..len], array, len, histogram, shift)
                } else {
                    Self::sort_pass(&array[..len], buffer, len, histogram, shift)
                }
            };
            if moved {
                in_buffer = !in_buffer;
            }
            shift += 8;
        }

        if in_buffer {
            array[..len].copy_from_slice(&self.buffer[..len]);
        }
    }

    /// One radix pass. Returns `false` when every value shares the same byte at
    /// `shift`, in which case nothing was written to `dest`.
    ///
    /// Equivalent to the private static `LSBRadixSorter.sort`.
    fn sort_pass(
        src: &[i32],
        dest: &mut [i32],
        len: usize,
        histogram: &mut [i32; LSB_HISTOGRAM_SIZE],
        shift: u32,
    ) -> bool {
        for &v in src.iter().take(len) {
            let b = ((v as u32) >> shift) as usize & 0xFF;
            histogram[b] += 1;
        }
        if histogram[0] == len as i32 {
            return false;
        }
        let mut accum = 0i32;
        for slot in histogram.iter_mut() {
            let count = *slot;
            *slot = accum;
            accum += count;
        }
        for &v in src.iter().take(len) {
            let b = ((v as u32) >> shift) as usize & 0xFF;
            dest[histogram[b] as usize] = v;
            histogram[b] += 1;
        }
        true
    }
}

/// Equivalent to the private static `LSBRadixSorter.insertionSort`.
fn insertion_sort_ints(array: &mut [i32], off: usize, len: usize) {
    let end = off + len;
    for i in (off + 1)..end {
        let mut j = i;
        while j > off {
            if array[j - 1] > array[j] {
                array.swap(j - 1, j);
            } else {
                break;
            }
            j -= 1;
        }
    }
}

// ---------------------------------------------------------------------------
// BytesRefComparator
// ---------------------------------------------------------------------------

/// A [`BytesRef`] comparator that [`StringSorter`] can drive with a radix sort.
///
/// Port of `org.apache.lucene.util.BytesRefComparator`.
pub trait BytesRefComparator {
    /// The maximum number of bytes to compare.
    ///
    /// Equivalent to the `comparedBytesCount` field.
    fn compared_bytes_count(&self) -> usize;

    /// Returns the unsigned byte to use for comparison at index `i`, or `-1`
    /// when every byte useful for comparisons is exhausted.
    ///
    /// May only be called with `i` in `[0, compared_bytes_count())`.
    fn byte_at(&self, r: &BytesRef, i: usize) -> i32;

    /// Compares two values.
    fn compare(&self, o1: &BytesRef, o2: &BytesRef) -> i32 {
        self.compare_from(o1, o2, 0)
    }

    /// Compares two values whose first `k` bytes are already known to be equal.
    fn compare_from(&self, o1: &BytesRef, o2: &BytesRef, k: usize) -> i32 {
        for i in k..self.compared_bytes_count() {
            let b1 = self.byte_at(o1, i);
            let b2 = self.byte_at(o2, i);
            if b1 != b2 {
                return b1 - b2;
            } else if b1 == -1 {
                break;
            }
        }
        0
    }
}

/// Compares [`BytesRef`]s in natural (unsigned lexicographic) order.
///
/// Port of `BytesRefComparator.NATURAL`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NaturalBytesRefComparator;

impl BytesRefComparator for NaturalBytesRefComparator {
    fn compared_bytes_count(&self) -> usize {
        i32::MAX as usize
    }

    fn byte_at(&self, r: &BytesRef, i: usize) -> i32 {
        if r.length <= i {
            -1
        } else {
            r.bytes[r.offset + i] as i32
        }
    }

    fn compare_from(&self, o1: &BytesRef, o2: &BytesRef, k: usize) -> i32 {
        compare_unsigned(&o1.slice()[k..], &o2.slice()[k..])
    }
}

/// `java.util.Arrays.compareUnsigned(byte[], int, int, byte[], int, int)`.
fn compare_unsigned(a: &[u8], b: &[u8]) -> i32 {
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return a[i] as i32 - b[i] as i32;
        }
    }
    a.len() as i32 - b.len() as i32
}

// ---------------------------------------------------------------------------
// StringSorter
// ---------------------------------------------------------------------------

/// The operations a [`StringSorter`] performs on the range it sorts.
///
/// Port of the abstract `get`/`swap` of
/// `org.apache.lucene.util.StringSorter`.
///
/// **Divergence from Lucene 10.5.0.** Java's signature is
/// `get(BytesRefBuilder, BytesRef, int)`: the caller supplies scratch that the
/// implementation fills, because a Java `BytesRef` is a view over a shared
/// `byte[]`. Rucene's [`BytesRef`] owns its buffer (see
/// [`crate::util::BytesRef`]), so the scratch pair has nothing to point at and
/// the accessor simply returns the value.
pub trait StringSorterOps {
    /// Returns the value stored at index `i`.
    fn get(&self, i: usize) -> BytesRef;

    /// Exchanges the values at `i` and `j`.
    fn swap(&mut self, i: usize, j: usize);
}

/// Which comparator a [`StringSorter`] was given.
///
/// Java decides between the radix sort and the fallback with
/// `cmp instanceof BytesRefComparator`; Rust has no such test, so the two cases
/// are an explicit enum.
#[derive(Clone, Copy)]
pub enum StringSorterComparator<'a> {
    /// A [`BytesRefComparator`], for which the radix sorter is used.
    Radix(&'a dyn BytesRefComparator),
    /// A plain comparator, for which the fallback sorter is used.
    Generic(&'a dyn Fn(&BytesRef, &BytesRef) -> i32),
}

/// A [`BytesRef`] sorter that uses an efficient radix sort when its comparator
/// is a [`BytesRefComparator`], and falls back to intro sort otherwise.
///
/// Port of `org.apache.lucene.util.StringSorter`.
pub struct StringSorter<'a> {
    cmp: StringSorterComparator<'a>,
}

impl<'a> StringSorter<'a> {
    /// Creates a sorter driven by `cmp`.
    pub fn new(cmp: StringSorterComparator<'a>) -> Self {
        Self { cmp }
    }

    /// Sorts `[from, to)`.
    pub fn sort<O: StringSorterOps + ?Sized>(&self, ops: &mut O, from: usize, to: usize) {
        match self.cmp {
            StringSorterComparator::Radix(cmp) => {
                let mut radix = MSBRadixSorter::new(cmp.compared_bytes_count());
                let mut adapter = StringRadixOps { ops, cmp };
                radix.sort(&mut adapter, from, to);
            }
            StringSorterComparator::Generic(cmp) => {
                string_fallback_sort(ops, cmp, from, to);
            }
        }
    }
}

/// Lucene's nested `StringSorter.MSBStringRadixSorter`.
struct StringRadixOps<'a, O: StringSorterOps + ?Sized> {
    ops: &'a mut O,
    cmp: &'a dyn BytesRefComparator,
}

impl<O: StringSorterOps + ?Sized> MSBRadixSorterOps for StringRadixOps<'_, O> {
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        self.cmp.byte_at(&self.ops.get(i), k)
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.ops.swap(i, j);
    }

    fn fallback_sort(&mut self, from: usize, to: usize, k: usize, _max_length: usize) {
        let cmp = self.cmp;
        let f = move |o1: &BytesRef, o2: &BytesRef| cmp.compare_from(o1, o2, k);
        string_fallback_sort(&mut *self.ops, &f, from, to);
    }
}

/// Lucene's `StringSorter.fallbackSorter(Comparator)`: an intro sorter that
/// reads values through [`StringSorterOps::get`].
pub fn string_fallback_sort<O: StringSorterOps + ?Sized>(
    ops: &mut O,
    cmp: &dyn Fn(&BytesRef, &BytesRef) -> i32,
    from: usize,
    to: usize,
) {
    let mut sorter = StringFallbackSorter {
        ops,
        cmp,
        pivot: None,
    };
    intro_sort(&mut sorter, from, to);
}

struct StringFallbackSorter<'a, O: StringSorterOps + ?Sized> {
    ops: &'a mut O,
    cmp: &'a dyn Fn(&BytesRef, &BytesRef) -> i32,
    pivot: Option<BytesRef>,
}

impl<O: StringSorterOps + ?Sized> PivotOps for StringFallbackSorter<'_, O> {
    fn swap(&mut self, i: usize, j: usize) {
        self.ops.swap(i, j);
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        (self.cmp)(&self.ops.get(i), &self.ops.get(j))
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot = Some(self.ops.get(i));
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        let pivot = self
            .pivot
            .as_ref()
            .expect("INVARIANT: set_pivot precedes compare_pivot");
        (self.cmp)(pivot, &self.ops.get(j))
    }
}

// ---------------------------------------------------------------------------
// StableStringSorter
// ---------------------------------------------------------------------------

/// The extra operations a [`StableStringSorter`] needs on top of
/// [`StringSorterOps`].
///
/// Port of the abstract `save`/`restore` of
/// `org.apache.lucene.util.StableStringSorter`.
pub trait StableStringSorterOps: StringSorterOps {
    /// Saves the `i`-th value into the `j`-th position of temporary storage.
    fn save(&mut self, i: usize, j: usize);

    /// Restores values `[i, j)` from temporary storage into the original one.
    fn restore(&mut self, i: usize, j: usize);
}

/// A stable [`StringSorter`].
///
/// Port of `org.apache.lucene.util.StableStringSorter`.
pub struct StableStringSorter<'a> {
    cmp: StringSorterComparator<'a>,
}

impl<'a> StableStringSorter<'a> {
    /// Creates a stable sorter driven by `cmp`.
    pub fn new(cmp: StringSorterComparator<'a>) -> Self {
        Self { cmp }
    }

    /// Sorts `[from, to)` stably.
    pub fn sort<O: StableStringSorterOps + ?Sized>(&self, ops: &mut O, from: usize, to: usize) {
        match self.cmp {
            StringSorterComparator::Radix(cmp) => {
                let mut radix = StableMSBRadixSorter::new(cmp.compared_bytes_count());
                let mut adapter = StableStringRadixOps { ops, cmp };
                radix.sort(&mut adapter, from, to);
            }
            StringSorterComparator::Generic(cmp) => {
                stable_string_fallback_sort(ops, cmp, from, to);
            }
        }
    }
}

/// The anonymous `StableMSBRadixSorter` returned by
/// `StableStringSorter.radixSorter`.
struct StableStringRadixOps<'a, O: StableStringSorterOps + ?Sized> {
    ops: &'a mut O,
    cmp: &'a dyn BytesRefComparator,
}

impl<O: StableStringSorterOps + ?Sized> MSBRadixSorterOps for StableStringRadixOps<'_, O> {
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        self.cmp.byte_at(&self.ops.get(i), k)
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.ops.swap(i, j);
    }

    fn reorder(
        &mut self,
        from: usize,
        to: usize,
        start_offsets: &mut [i32],
        end_offsets: &[i32],
        k: usize,
    ) {
        stable_reorder(self, from, to, start_offsets, end_offsets, k);
    }

    fn fallback_sort(&mut self, from: usize, to: usize, k: usize, _max_length: usize) {
        let cmp = self.cmp;
        let f = move |o1: &BytesRef, o2: &BytesRef| cmp.compare_from(o1, o2, k);
        stable_string_fallback_sort(&mut *self.ops, &f, from, to);
    }
}

impl<O: StableStringSorterOps + ?Sized> StableMSBRadixSorterOps for StableStringRadixOps<'_, O> {
    fn save(&mut self, i: usize, j: usize) {
        self.ops.save(i, j);
    }

    fn restore(&mut self, i: usize, j: usize) {
        self.ops.restore(i, j);
    }
}

/// Lucene's `StableStringSorter.fallbackSorter(Comparator)`: a [`MergeSorter`]
/// reading values through [`StringSorterOps::get`].
pub fn stable_string_fallback_sort<O: StableStringSorterOps + ?Sized>(
    ops: &mut O,
    cmp: &dyn Fn(&BytesRef, &BytesRef) -> i32,
    from: usize,
    to: usize,
) {
    let mut sorter = StableStringFallbackSorter {
        ops,
        cmp,
        pivot_index: 0,
    };
    sorter.merge_sorter_sort(from, to);
}

struct StableStringFallbackSorter<'a, O: StableStringSorterOps + ?Sized> {
    ops: &'a mut O,
    cmp: &'a dyn Fn(&BytesRef, &BytesRef) -> i32,
    pivot_index: usize,
}

impl<O: StableStringSorterOps + ?Sized> PivotOps for StableStringFallbackSorter<'_, O> {
    fn swap(&mut self, i: usize, j: usize) {
        self.ops.swap(i, j);
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        (self.cmp)(&self.ops.get(i), &self.ops.get(j))
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot_index = i;
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.compare(self.pivot_index, j)
    }
}

impl<O: StableStringSorterOps + ?Sized> Sorter for StableStringFallbackSorter<'_, O> {
    fn sort(&mut self, from: usize, to: usize) {
        self.merge_sorter_sort(from, to);
    }
}

impl<O: StableStringSorterOps + ?Sized> MergeSorter for StableStringFallbackSorter<'_, O> {
    fn save(&mut self, i: usize, j: usize) {
        self.ops.save(i, j);
    }

    fn restore(&mut self, i: usize, j: usize) {
        self.ops.restore(i, j);
    }
}
