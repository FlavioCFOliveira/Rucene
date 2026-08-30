//! Partial sorting: put the `k`-th element of a range in its place and leave
//! everything smaller before it and everything larger after it.
//!
//! Ports `org.apache.lucene.util.Selector` and its two implementations,
//! `IntroSelector` and `RadixSelector`. Together they are what
//! `MutablePointTreeReaderUtils.partition` uses to split a BKD node, and the
//! **order they leave each side in is observable** in the index files: the
//! leaf that follows reads the point sitting at the start of the range when it
//! chooses which dimension to compress on. A selection that produced the right
//! two sets in a different arrangement would therefore write different bytes,
//! so these are ports of the algorithms, not of their contracts.
//!
//! # Shape of the port
//!
//! Java expresses both as abstract classes whose abstract methods the caller
//! fills in — `swap`, `byteAt`, `setPivot`, `comparePivot`. Rust has no
//! inheritance, so each abstract class becomes a trait carrying exactly those
//! methods and the algorithm becomes a function over it. `RadixSelector` keeps
//! its per-instance scratch (the histogram and the common-prefix buffer) in a
//! struct, as Java does, so a caller that selects repeatedly allocates once.
//!
//! # The shuffle guard: the one place where **Java** is the unreproducible one
//!
//! `IntroSelector.select` protects itself against adversarial input: once the
//! recursion has gone `2 * log2(size)` levels without narrowing, it shuffles
//! the range once and carries on (`IntroSelector.java:57-60`, shuffling at `:206-214`). Java shuffles
//! with a `java.util.SplittableRandom` built with **no seed**, so two runs of
//! Java over identical input can produce different output once that branch is
//! taken — and, downstream, different index bytes. It is Java that has no
//! reproducible behaviour there, not this port, and no byte-identity contract
//! can cover it in either direction.
//!
//! This port shuffles from the fixed sequence below instead. Where the original
//! has nothing defined to reproduce, a deterministic choice is the only one that
//! can be tested at all.
//!
//! **Measured: the branch does not fire.** Instrumenting it and running the 288
//! flushes that `crate::index::point_values_writer` compares against Lucene
//! 10.5.0 reached it **zero times**. That is an observation about the measured
//! corpus and nothing more: no argument is offered here that the budget of
//! `2 * log2(size)` cannot be exhausted, only that no input in that corpus
//! exhausted it. Everything outside this branch is reproduced exactly.

use crate::util::BytesRef;

/// Number of entries below which `RadixSelector` gives up on radix and calls
/// the fallback. `RadixSelector.LENGTH_THRESHOLD`.
const LENGTH_THRESHOLD: usize = 100;

/// Recursion depth after which `RadixSelector` gives up on radix and calls the
/// fallback, because long common prefixes make radix selection slower than
/// introselect. `RadixSelector.LEVEL_THRESHOLD`.
const LEVEL_THRESHOLD: usize = 8;

/// 256 byte values plus one slot meaning "the string ended here".
/// `RadixSelector.HISTOGRAM_SIZE`.
const HISTOGRAM_SIZE: usize = 257;

/// Range size at or below which `IntroSelector` and `IntroSorter` pick their
/// pivot from a single median rather than a median of medians.
/// `IntroSorter.SINGLE_MEDIAN_THRESHOLD`.
const SINGLE_MEDIAN_THRESHOLD: isize = 40;

/// Range size at or below which `IntroSorter` finishes with insertion sort.
/// `Sorter.INSERTION_SORT_THRESHOLD`.
const INSERTION_SORT_THRESHOLD: isize = 16;

/// `MathUtil.log(x, 2)`: the position of the highest set bit, and `0` for a
/// non-positive argument.
fn log2(x: usize) -> u32 {
    if x == 0 {
        0
    } else {
        usize::BITS - 1 - x.leading_zeros()
    }
}

// -----------------------------------------------------------------------------
// IntroSelector
// -----------------------------------------------------------------------------

/// The operations [`intro_select`] and [`intro_sort`] perform on the range they
/// work over.
///
/// Java declares the same four primitives twice, once on
/// `org.apache.lucene.util.Selector` plus `IntroSelector` and once on
/// `org.apache.lucene.util.Sorter` plus `IntroSorter`, because a Java class can
/// only extend one of them. Rust has no such constraint, so this port declares
/// them once; a type that both sorts and selects implements one trait instead
/// of two identical ones.
pub trait PivotOps {
    /// Exchanges the elements at `i` and `j`.
    fn swap(&mut self, i: usize, j: usize);

    /// Remembers the element at `i` as the pivot.
    fn set_pivot(&mut self, i: usize);

    /// Compares the remembered pivot with the element at `j`, returning a
    /// negative number, zero or a positive number as in `Comparator.compare`.
    fn compare_pivot(&mut self, j: usize) -> i32;

    /// Compares the elements at `i` and `j`.
    ///
    /// Java's default implementation, which most callers keep: set the pivot to
    /// `i` and compare it with `j`.
    fn compare(&mut self, i: usize, j: usize) -> i32 {
        self.set_pivot(i);
        self.compare_pivot(j)
    }
}

/// Puts the `k`-th element of `[from, to)` in its place.
///
/// Equivalent to `IntroSelector.select(int, int, int)`
/// (`IntroSelector.java:42-46`), which computes the recursion budget and calls
/// the four-argument form.
///
/// # Panics
///
/// Panics when `k` is outside `[from, to)`, which is `Selector.checkArgs`
/// throwing `IllegalArgumentException`: it is a caller bug, not a property of
/// the data.
pub fn intro_select(ops: &mut dyn PivotOps, from: usize, to: usize, k: usize) {
    check_args(from, to, k);
    let max_depth = 2 * log2(to - from) as i64;
    intro_select_bounded(ops, from, to, k, max_depth);
}

fn check_args(from: usize, to: usize, k: usize) {
    assert!(k >= from, "k must be >= from");
    assert!(k < to, "k must be < to");
}

/// The body of `IntroSelector.select(int, int, int, int)`
/// (`IntroSelector.java:49-147`): medians-of-medians pivot selection,
/// Bentley-McIlroy three-way partitioning, and a specialised sort for the last
/// three entries.
fn intro_select_bounded(ops: &mut dyn PivotOps, from: usize, to: usize, k: usize, max_depth: i64) {
    // Java's indices are `int` and the partition loops let them step one past
    // each end of the range before the guards catch them, so they are signed
    // here too rather than `usize`.
    let (mut from, mut to) = (from as isize, to as isize);
    let k = k as isize;
    let mut max_depth = max_depth;

    let mut size = to - from;
    while size > 3 {
        max_depth -= 1;
        if max_depth == -1 {
            // See the module documentation: this is the branch in which Java
            // itself is not reproducible.
            shuffle(ops, from, to);
        }

        let last = to - 1;
        let mid = ((from + last) as usize >> 1) as isize;
        let pivot = if size <= SINGLE_MEDIAN_THRESHOLD {
            // A single median around the middle element. Not the median of
            // [from, mid, last]: that hurts a descending range badly in
            // combination with the three-way partitioning.
            let range = size >> 2;
            median(ops, mid - range, mid, mid + range)
        } else {
            // A variant of Tukey's ninther. When `k` is near a boundary the
            // lowest or highest median is taken instead of the middle one,
            // which is the interpolation-search idea.
            let range = size >> 3;
            let double_range = range << 1;
            let median_first = median(ops, from, from + range, from + double_range);
            let median_middle = median(ops, mid - range, mid, mid + range);
            let median_last = median(ops, last - double_range, last - range, last);
            if k - from < range {
                min3(ops, median_first, median_middle, median_last)
            } else if to - k <= range {
                max3(ops, median_first, median_middle, median_last)
            } else {
                median(ops, median_first, median_middle, median_last)
            }
        };

        // Bentley-McIlroy three-way partitioning.
        ops.set_pivot(pivot as usize);
        ops.swap(from as usize, pivot as usize);
        let mut i = from;
        let mut j = to;
        let mut p = from + 1;
        let mut q = last;
        loop {
            let mut left_cmp;
            loop {
                i += 1;
                left_cmp = ops.compare_pivot(i as usize);
                if left_cmp <= 0 {
                    break;
                }
            }
            let mut right_cmp;
            loop {
                j -= 1;
                right_cmp = ops.compare_pivot(j as usize);
                if right_cmp >= 0 {
                    break;
                }
            }
            if i >= j {
                if i == j && right_cmp == 0 {
                    ops.swap(i as usize, p as usize);
                }
                break;
            }
            ops.swap(i as usize, j as usize);
            if right_cmp == 0 {
                ops.swap(i as usize, p as usize);
                p += 1;
            }
            if left_cmp == 0 {
                ops.swap(j as usize, q as usize);
                q -= 1;
            }
        }
        i = j + 1;
        let mut l = from;
        while l < p {
            ops.swap(l as usize, j as usize);
            l += 1;
            j -= 1;
        }
        let mut l = last;
        while l > q {
            ops.swap(l as usize, i as usize);
            l -= 1;
            i += 1;
        }

        // Keep only the side that holds the k-th element.
        if k <= j {
            to = j + 1;
        } else if k >= i {
            from = i;
        } else {
            return;
        }
        size = to - from;
    }

    match size {
        2 => {
            if ops.compare(from as usize, (from + 1) as usize) > 0 {
                ops.swap(from as usize, (from + 1) as usize);
            }
        }
        3 => sort3(ops, from),
        _ => {}
    }
}

// -----------------------------------------------------------------------------
// IntroSorter
// -----------------------------------------------------------------------------

/// Sorts `[from, to)`.
///
/// Equivalent to `IntroSorter.sort(int, int)` (`IntroSorter.java:41-45`), which
/// computes the recursion budget and calls the three-argument form.
///
/// **This sort is not stable, and that is the point.** Java sorts a BKD leaf
/// with an `IntroSorter` whose comparator looks at the sorted dimension, then
/// the *non-index* data dimensions, then the doc ID
/// (`MutablePointTreeReaderUtils.java:88-140`). That comparator ignores the
/// **other index dimensions**, so two points of the same document can tie while
/// still differing in the bytes the leaf writes. Which of them lands first is
/// decided by the algorithm, not by the comparator — so a stable sort, or any
/// other correct sort, writes a different `.kdd`.
pub fn intro_sort(ops: &mut dyn PivotOps, from: usize, to: usize) {
    assert!(from <= to, "from must be <= to");
    if to - from <= 1 {
        return;
    }
    let max_depth = 2 * log2(to - from) as i64;
    intro_sort_bounded(ops, from as isize, to as isize, max_depth);
}

/// The body of `IntroSorter.sort(int, int, int)` (`IntroSorter.java:54-129`).
///
/// Same pivot selection and same Bentley-McIlroy three-way partitioning as
/// [`intro_select`], but it recurses into the *smaller* partition and loops on
/// the larger one, and it falls back to heap sort — not to a shuffle — when the
/// recursion budget runs out. Nothing here is unreproducible.
fn intro_sort_bounded(ops: &mut dyn PivotOps, from: isize, to: isize, max_depth: i64) {
    let mut from = from;
    let mut to = to;
    let mut max_depth = max_depth;

    let mut size = to - from;
    while size > INSERTION_SORT_THRESHOLD {
        max_depth -= 1;
        if max_depth < 0 {
            heap_sort(ops, from, to);
            return;
        }

        let last = to - 1;
        let mid = ((from + last) as usize >> 1) as isize;
        let pivot = if size <= SINGLE_MEDIAN_THRESHOLD {
            let range = size >> 2;
            median(ops, mid - range, mid, mid + range)
        } else {
            // Tukey's ninther. Unlike `IntroSelector`, the sorter has no `k` to
            // lean towards, so it always takes the median of the three medians.
            let range = size >> 3;
            let double_range = range << 1;
            let median_first = median(ops, from, from + range, from + double_range);
            let median_middle = median(ops, mid - range, mid, mid + range);
            let median_last = median(ops, last - double_range, last - range, last);
            median(ops, median_first, median_middle, median_last)
        };

        ops.set_pivot(pivot as usize);
        ops.swap(from as usize, pivot as usize);
        let mut i = from;
        let mut j = to;
        let mut p = from + 1;
        let mut q = last;
        loop {
            let mut left_cmp;
            loop {
                i += 1;
                left_cmp = ops.compare_pivot(i as usize);
                if left_cmp <= 0 {
                    break;
                }
            }
            let mut right_cmp;
            loop {
                j -= 1;
                right_cmp = ops.compare_pivot(j as usize);
                if right_cmp >= 0 {
                    break;
                }
            }
            if i >= j {
                if i == j && right_cmp == 0 {
                    ops.swap(i as usize, p as usize);
                }
                break;
            }
            ops.swap(i as usize, j as usize);
            if right_cmp == 0 {
                ops.swap(i as usize, p as usize);
                p += 1;
            }
            if left_cmp == 0 {
                ops.swap(j as usize, q as usize);
                q -= 1;
            }
        }
        i = j + 1;
        let mut k = from;
        while k < p {
            ops.swap(k as usize, j as usize);
            k += 1;
            j -= 1;
        }
        let mut k = last;
        while k > q {
            ops.swap(k as usize, i as usize);
            k -= 1;
            i += 1;
        }

        // Recurse on the smaller partition, loop on the larger one.
        if j - from < last - i {
            intro_sort_bounded(ops, from, j + 1, max_depth);
            from = i;
        } else {
            intro_sort_bounded(ops, i, to, max_depth);
            to = j + 1;
        }
        size = to - from;
    }

    insertion_sort(ops, from, to);
}

/// `Sorter.insertionSort` (`Sorter.java:240-252`). Stable, and used only for
/// ranges of 16 elements or fewer.
fn insertion_sort(ops: &mut dyn PivotOps, from: isize, to: isize) {
    let mut i = from + 1;
    while i < to {
        let mut current = i;
        i += 1;
        loop {
            let previous = current - 1;
            if ops.compare(previous as usize, current as usize) <= 0 {
                break;
            }
            ops.swap(previous as usize, current as usize);
            if previous == from {
                break;
            }
            current = previous;
        }
    }
}

/// `Sorter.heapSort` (`Sorter.java:259-271`), the deterministic fallback
/// `IntroSorter` takes when its recursion budget runs out.
fn heap_sort(ops: &mut dyn PivotOps, from: isize, to: isize) {
    if to - from <= 1 {
        return;
    }
    heapify(ops, from, to);
    let mut end = to - 1;
    while end > from {
        ops.swap(from as usize, end as usize);
        sift_down(ops, from, from, end);
        end -= 1;
    }
}

/// `Sorter.heapify` (`Sorter.java:270-274`).
fn heapify(ops: &mut dyn PivotOps, from: isize, to: isize) {
    let mut i = heap_parent(from, to - 1);
    while i >= from {
        sift_down(ops, i, from, to);
        i -= 1;
    }
}

/// `Sorter.siftDown` (`Sorter.java:276-294`).
fn sift_down(ops: &mut dyn PivotOps, i: isize, from: isize, to: isize) {
    let mut i = i;
    loop {
        let left_child = heap_child(from, i);
        if left_child >= to {
            break;
        }
        let right_child = left_child + 1;
        if ops.compare(i as usize, left_child as usize) < 0 {
            if right_child < to && ops.compare(left_child as usize, right_child as usize) < 0 {
                ops.swap(i as usize, right_child as usize);
                i = right_child;
            } else {
                ops.swap(i as usize, left_child as usize);
                i = left_child;
            }
        } else if right_child < to && ops.compare(i as usize, right_child as usize) < 0 {
            ops.swap(i as usize, right_child as usize);
            i = right_child;
        } else {
            break;
        }
    }
}

/// `Sorter.heapParent` (`Sorter.java:296-298`).
fn heap_parent(from: isize, i: isize) -> isize {
    (((i - 1 - from) as usize) >> 1) as isize + from
}

/// `Sorter.heapChild` (`Sorter.java:300-302`).
fn heap_child(from: isize, i: isize) -> isize {
    ((i - from) << 1) + 1 + from
}

/// Index of the smallest of the three elements. `IntroSelector.min`.
fn min3(ops: &mut dyn PivotOps, i: isize, j: isize, k: isize) -> isize {
    if ops.compare(i as usize, j as usize) <= 0 {
        if ops.compare(i as usize, k as usize) <= 0 {
            i
        } else {
            k
        }
    } else if ops.compare(j as usize, k as usize) <= 0 {
        j
    } else {
        k
    }
}

/// Index of the largest of the three elements. `IntroSelector.max`.
fn max3(ops: &mut dyn PivotOps, i: isize, j: isize, k: isize) -> isize {
    if ops.compare(i as usize, j as usize) <= 0 {
        if ops.compare(j as usize, k as usize) < 0 {
            k
        } else {
            j
        }
    } else if ops.compare(i as usize, k as usize) < 0 {
        k
    } else {
        i
    }
}

/// Index of the median of the three elements. `IntroSelector.median`, itself a
/// copy of `IntroSorter.median`.
fn median(ops: &mut dyn PivotOps, i: isize, j: isize, k: isize) -> isize {
    if ops.compare(i as usize, j as usize) < 0 {
        if ops.compare(j as usize, k as usize) <= 0 {
            return j;
        }
        return if ops.compare(i as usize, k as usize) < 0 {
            k
        } else {
            i
        };
    }
    if ops.compare(j as usize, k as usize) >= 0 {
        return j;
    }
    if ops.compare(i as usize, k as usize) < 0 {
        i
    } else {
        k
    }
}

/// Sorts the three elements at `from`, `from + 1` and `from + 2` with at most
/// three comparisons. `IntroSelector.sort3`.
fn sort3(ops: &mut dyn PivotOps, from: isize) {
    let mid = from + 1;
    let last = from + 2;
    if ops.compare(from as usize, mid as usize) <= 0 {
        if ops.compare(mid as usize, last as usize) > 0 {
            ops.swap(mid as usize, last as usize);
            if ops.compare(from as usize, mid as usize) > 0 {
                ops.swap(from as usize, mid as usize);
            }
        }
    } else if ops.compare(mid as usize, last as usize) >= 0 {
        ops.swap(from as usize, last as usize);
    } else {
        ops.swap(from as usize, mid as usize);
        if ops.compare(mid as usize, last as usize) > 0 {
            ops.swap(mid as usize, last as usize);
        }
    }
}

/// Fisher-Yates over `[from, to)`, run once when the recursion budget is spent.
///
/// `IntroSelector.shuffle` draws from an unseeded `SplittableRandom`; see the
/// module documentation for why this port draws from a fixed sequence instead.
fn shuffle(ops: &mut dyn PivotOps, from: isize, to: isize) {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut i = to - 1;
    while i > from {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let span = (i - from + 1) as u64;
        let pick =
            from + (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33).wrapping_rem(span) as isize;
        ops.swap(i as usize, pick as usize);
        i -= 1;
    }
}

// -----------------------------------------------------------------------------
// RadixSelector
// -----------------------------------------------------------------------------

/// The operations [`RadixSelector`] performs on the range it is selecting over.
///
/// Equivalent to the abstract methods of `org.apache.lucene.util.RadixSelector`
/// together with `Selector.swap`.
pub trait RadixSelectorOps {
    /// The `k`-th byte of the element at `i`, as a value in `0..=255`, or `-1`
    /// when that element is shorter than `k + 1` bytes.
    ///
    /// Equivalent to `RadixSelector.byteAt`.
    fn byte_at(&self, i: usize, k: usize) -> i32;

    /// Exchanges the elements at `i` and `j`.
    fn swap(&mut self, i: usize, j: usize);

    /// Selects `[from, to)` around `k` comparing from byte `d` onwards, for a
    /// range too short or too deep for radix selection to pay off.
    ///
    /// Equivalent to `RadixSelector.getFallbackSelector(d).select(from, to, k)`.
    /// Java builds a fresh `IntroSelector` per call and lets the caller
    /// override which one; this port asks the caller to run it, which is the
    /// same seam without the object.
    fn fallback_select(&mut self, from: usize, to: usize, k: usize, d: usize);
}

/// Selects by comparing elements byte by byte, most significant byte first.
///
/// Equivalent to `org.apache.lucene.util.RadixSelector`. Holds the histogram
/// and common-prefix scratch across calls, as the Java instance does.
pub struct RadixSelector {
    max_length: usize,
    histogram: Vec<i32>,
    common_prefix: Vec<i32>,
}

impl RadixSelector {
    /// Creates a selector over elements of at most `max_length` bytes.
    ///
    /// Equivalent to `RadixSelector(int maxLength)`; the common-prefix buffer
    /// is capped at 24 bytes exactly as Java caps it.
    pub fn new(max_length: usize) -> Self {
        Self {
            max_length,
            histogram: vec![0i32; HISTOGRAM_SIZE],
            common_prefix: vec![0i32; max_length.min(24)],
        }
    }

    /// Puts the `k`-th element of `[from, to)` in its place.
    ///
    /// Equivalent to `RadixSelector.select(int, int, int)`.
    ///
    /// # Panics
    ///
    /// Panics when `k` is outside `[from, to)`, which is `Selector.checkArgs`
    /// throwing `IllegalArgumentException`.
    pub fn select(&mut self, ops: &mut dyn RadixSelectorOps, from: usize, to: usize, k: usize) {
        check_args(from, to, k);
        self.select_at(ops, from, to, k, 0, 0);
    }

    /// `RadixSelector.select(int, int, int, int, int)`: radix while the range
    /// is long enough and the recursion shallow enough, introselect otherwise.
    fn select_at(
        &mut self,
        ops: &mut dyn RadixSelectorOps,
        from: usize,
        to: usize,
        k: usize,
        d: usize,
        l: usize,
    ) {
        if to - from <= LENGTH_THRESHOLD || l >= LEVEL_THRESHOLD {
            ops.fallback_select(from, to, k, d);
        } else {
            self.radix_select(ops, from, to, k, d, l);
        }
    }

    /// `RadixSelector.radixSelect`: build the histogram of byte `d`, move the
    /// bucket that holds `k` into place, and recurse into it.
    fn radix_select(
        &mut self,
        ops: &mut dyn RadixSelectorOps,
        from: usize,
        to: usize,
        k: usize,
        d: usize,
        l: usize,
    ) {
        self.histogram.iter_mut().for_each(|slot| *slot = 0);
        let common_prefix_length = self.common_prefix_length_and_histogram(ops, from, to, d);
        if common_prefix_length > 0 {
            // Either there is nothing left to compare, or every element fell
            // into the "string ended" bucket, in which case they are all equal
            // from here on and there is nothing to select.
            if d + common_prefix_length < self.max_length && self.histogram[0] < (to - from) as i32
            {
                self.radix_select(ops, from, to, k, d + common_prefix_length, l);
            }
            return;
        }

        let mut bucket_from = from;
        for bucket in 0..HISTOGRAM_SIZE {
            let bucket_to = bucket_from + self.histogram[bucket] as usize;
            if bucket_to > k {
                partition(ops, from, to, bucket as i32, bucket_from, bucket_to, d);
                // Bucket 0 holds the elements that ended before byte `d`, so
                // they are all equal and need no further work.
                if bucket != 0 && d + 1 < self.max_length {
                    self.select_at(ops, bucket_from, bucket_to, k, d + 1, l + 1);
                }
                return;
            }
            bucket_from = bucket_to;
        }
        unreachable!("the bucket holding k must exist");
    }

    /// `RadixSelector.computeCommonPrefixLengthAndBuildHistogram`.
    ///
    /// Java splits this across four methods to work around a JVM crash
    /// (apache/lucene#12898); that is a codegen workaround, not behaviour, so
    /// this port keeps it in one place.
    fn common_prefix_length_and_histogram(
        &mut self,
        ops: &dyn RadixSelectorOps,
        from: usize,
        to: usize,
        k: usize,
    ) -> usize {
        // The prefix of the first element, up to the scratch buffer's length.
        let mut common_prefix_length = self.common_prefix.len().min(self.max_length - k);
        for j in 0..common_prefix_length {
            let b = ops.byte_at(from, k + j);
            self.common_prefix[j] = b;
            if b == -1 {
                common_prefix_length = j + 1;
                break;
            }
        }

        // Java's inner loop re-reads `commonPrefixLength` on every iteration
        // because the body shortens it, so the index is explicit here rather
        // than a range that would be captured once.
        let mut i = from + 1;
        'outer: while i < to {
            let mut j = 0;
            while j < common_prefix_length {
                let b = ops.byte_at(i, k + j);
                if b != self.common_prefix[j] {
                    common_prefix_length = j;
                    if common_prefix_length == 0 {
                        // No common prefix at all: seed the histogram with what
                        // is already known and count the rest.
                        self.histogram[(self.common_prefix[0] + 1) as usize] = (i - from) as i32;
                        self.histogram[(b + 1) as usize] = 1;
                        break 'outer;
                    }
                    break;
                }
                j += 1;
            }
            i += 1;
        }

        if i < to {
            for slot in (i + 1)..to {
                let bucket = (ops.byte_at(slot, k) + 1) as usize;
                self.histogram[bucket] += 1;
            }
        } else {
            self.histogram[(self.common_prefix[0] + 1) as usize] = (to - from) as i32;
        }
        common_prefix_length
    }
}

/// `RadixSelector.partition`: moves every element of `bucket` into
/// `[bucket_from, bucket_to)`, leaving smaller buckets before it and larger
/// ones after it.
///
/// The arrangement **within** each side is what this loop happens to produce,
/// and it is observable downstream; see the module documentation.
fn partition(
    ops: &mut dyn RadixSelectorOps,
    from: usize,
    to: usize,
    bucket: i32,
    bucket_from: usize,
    bucket_to: usize,
    d: usize,
) {
    let mut left = from;
    let mut right = to - 1;
    let mut slot = bucket_from;

    loop {
        let mut left_bucket = ops.byte_at(left, d) + 1;
        let mut right_bucket = ops.byte_at(right, d) + 1;

        while left_bucket <= bucket && left < bucket_from {
            if left_bucket == bucket {
                ops.swap(left, slot);
                slot += 1;
            } else {
                left += 1;
            }
            left_bucket = ops.byte_at(left, d) + 1;
        }

        while right_bucket >= bucket && right >= bucket_to {
            if right_bucket == bucket {
                ops.swap(right, slot);
                slot += 1;
            } else {
                right -= 1;
            }
            right_bucket = ops.byte_at(right, d) + 1;
        }

        if left < bucket_from && right >= bucket_to {
            ops.swap(left, right);
            left += 1;
            right -= 1;
        } else {
            debug_assert_eq!(left, bucket_from);
            debug_assert_eq!(right, bucket_to - 1);
            break;
        }
    }
}

// -----------------------------------------------------------------------------
// A ready-made selector over byte strings, used by the tests
// -----------------------------------------------------------------------------

/// Selects over a slice of byte strings, comparing them lexicographically by
/// unsigned byte and then by their position in the original slice.
///
/// Exists so the algorithms above can be tested on their own rather than only
/// through the BKD writer that consumes them.
pub struct BytesRefSelector<'a> {
    values: &'a mut [BytesRef],
    /// How many leading bytes of each value take part in the comparison. Bytes
    /// beyond it ride along untouched, the way a BKD point's doc ID does.
    key_length: usize,
    pivot: Vec<u8>,
    fallback_d: usize,
}

impl<'a> BytesRefSelector<'a> {
    /// Wraps `values`, which is reordered in place and compared whole.
    pub fn new(values: &'a mut [BytesRef]) -> Self {
        let key_length = values.iter().map(|v| v.slice().len()).max().unwrap_or(0);
        Self::with_key_length(values, key_length)
    }

    /// Wraps `values`, comparing only their first `key_length` bytes.
    pub fn with_key_length(values: &'a mut [BytesRef], key_length: usize) -> Self {
        Self {
            values,
            key_length,
            pivot: Vec::new(),
            fallback_d: 0,
        }
    }

    /// The longest value, which is the `max_length` a [`RadixSelector`] over
    /// these values needs.
    pub fn max_length(&self) -> usize {
        self.values
            .iter()
            .map(|v| v.slice().len())
            .max()
            .unwrap_or(0)
    }
}

impl RadixSelectorOps for BytesRefSelector<'_> {
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        let bytes = self.values[i].slice();
        if k < bytes.len().min(self.key_length) {
            i32::from(bytes[k])
        } else {
            -1
        }
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.values.swap(i, j);
    }

    fn fallback_select(&mut self, from: usize, to: usize, k: usize, d: usize) {
        self.fallback_d = d;
        intro_select(self, from, to, k);
    }
}

impl PivotOps for BytesRefSelector<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.values.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        let key = &self.values[i].slice()[..self.key_length.min(self.values[i].slice().len())];
        let d = self.fallback_d.min(key.len());
        self.pivot.clear();
        self.pivot.extend_from_slice(&key[d..]);
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        let key = &self.values[j].slice()[..self.key_length.min(self.values[j].slice().len())];
        let d = self.fallback_d.min(key.len());
        let other = &key[d..];
        match self.pivot.as_slice().cmp(other) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A selector over `i32`s, the smallest thing that exercises
    /// [`intro_select`] on its own.
    struct IntSelector<'a> {
        values: &'a mut [i32],
        pivot: i32,
    }

    impl PivotOps for IntSelector<'_> {
        fn swap(&mut self, i: usize, j: usize) {
            self.values.swap(i, j);
        }

        fn set_pivot(&mut self, i: usize) {
            self.pivot = self.values[i];
        }

        fn compare_pivot(&mut self, j: usize) -> i32 {
            self.pivot.cmp(&self.values[j]) as i32
        }
    }

    /// A deterministic pseudo-random sequence, mirroring the xorshift64* the
    /// portability probes use so the two corpora are comparable.
    fn xorshift(seed: u64, count: usize, modulus: i32) -> Vec<i32> {
        let mut state = seed;
        (0..count)
            .map(|_| {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                let value = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as i32;
                if modulus <= 0 {
                    value
                } else {
                    value % modulus
                }
            })
            .collect()
    }

    fn assert_selected(values: &[i32], k: usize, expected: &[i32]) {
        assert_eq!(
            values[k], expected[k],
            "the k-th element must be the k-th smallest"
        );
        for (index, value) in values.iter().enumerate() {
            if index < k {
                assert!(
                    *value <= values[k],
                    "index {index} must not exceed values[k]"
                );
            } else if index > k {
                assert!(
                    *value >= values[k],
                    "index {index} must not be below values[k]"
                );
            }
        }
    }

    #[test]
    fn intro_select_puts_the_kth_element_in_its_place() {
        // Every size across the three code paths: the tiny switch (0..=3), the
        // single-median pivot (<= 40) and the ninther (> 40).
        for count in [1usize, 2, 3, 4, 7, 40, 41, 100, 513] {
            for seed in [1u64, 2, 12345] {
                for modulus in [0i32, 10, 1000] {
                    let values = xorshift(seed, count, modulus);
                    let mut expected = values.clone();
                    expected.sort_unstable();
                    for k in [0usize, count / 3, count / 2, count - 1] {
                        let mut actual = values.clone();
                        {
                            let mut ops = IntSelector {
                                values: &mut actual,
                                pivot: 0,
                            };
                            intro_select(&mut ops, 0, count, k);
                        }
                        assert_selected(&actual, k, &expected);
                        let mut sorted = actual.clone();
                        sorted.sort_unstable();
                        assert_eq!(
                            sorted, expected,
                            "count={count} seed={seed} modulus={modulus} k={k}: \
                             selection must permute, never lose or invent values"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn intro_select_handles_a_sub_range() {
        // `from` and `to` are not the whole slice: everything outside must be
        // left untouched, which is what the BKD recursion relies on.
        let mut values: Vec<i32> = (0..20).rev().collect();
        let before_head = values[..5].to_vec();
        let before_tail = values[15..].to_vec();
        {
            let mut ops = IntSelector {
                values: &mut values,
                pivot: 0,
            };
            intro_select(&mut ops, 5, 15, 9);
        }
        assert_eq!(&values[..5], &before_head[..], "the head must not move");
        assert_eq!(&values[15..], &before_tail[..], "the tail must not move");
        let mut middle = values[5..15].to_vec();
        middle.sort_unstable();
        assert_eq!(values[9], middle[4], "the k-th element of the sub-range");
    }

    #[test]
    fn intro_select_survives_a_constant_range() {
        // Every element equal is the case the three-way partitioning exists
        // for: the loop must terminate rather than swap forever.
        let mut values = vec![7i32; 300];
        {
            let mut ops = IntSelector {
                values: &mut values,
                pivot: 0,
            };
            intro_select(&mut ops, 0, 300, 150);
        }
        assert!(values.iter().all(|value| *value == 7));
    }

    fn bytes(values: &[&[u8]]) -> Vec<BytesRef> {
        values
            .iter()
            .map(|value| BytesRef::new(value.to_vec()))
            .collect()
    }

    #[test]
    fn radix_select_puts_the_kth_string_in_its_place() {
        // Long enough to stay on the radix path (over 100 entries), with a long
        // shared prefix so the common-prefix shortcut runs, and varying lengths
        // so `byteAt` returns -1 for some.
        let values: Vec<BytesRef> = (0..300)
            .map(|i: i32| {
                let mut value = b"common-prefix-".to_vec();
                value.extend_from_slice(format!("{:04}", (i * 7919) % 997).as_bytes());
                if i % 5 == 0 {
                    value.truncate(value.len() - 2);
                }
                BytesRef::new(value)
            })
            .collect();
        let mut expected: Vec<Vec<u8>> = values.iter().map(|v| v.slice().to_vec()).collect();
        expected.sort();

        for k in [0usize, 1, 99, 100, 101, 150, 299] {
            let mut actual = values.clone();
            let max_length = actual.iter().map(|v| v.slice().len()).max().unwrap();
            {
                let mut ops = BytesRefSelector::new(&mut actual);
                RadixSelector::new(max_length).select(&mut ops, 0, 300, k);
            }
            assert_eq!(
                actual[k].slice(),
                expected[k].as_slice(),
                "k={k}: the k-th element must be the k-th smallest"
            );
            for (index, value) in actual.iter().enumerate() {
                if index < k {
                    assert!(value.slice() <= actual[k].slice(), "k={k} index={index}");
                } else if index > k {
                    assert!(value.slice() >= actual[k].slice(), "k={k} index={index}");
                }
            }
            let mut permuted: Vec<Vec<u8>> = actual.iter().map(|v| v.slice().to_vec()).collect();
            permuted.sort();
            assert_eq!(permuted, expected, "k={k}: selection must permute");
        }
    }

    #[test]
    fn radix_select_falls_back_below_the_length_threshold() {
        // 100 entries or fewer never reach the histogram at all: the fallback
        // is the whole algorithm there, and it must still be correct.
        let mut values = bytes(&[b"delta", b"alpha", b"charlie", b"bravo", b"echo"]);
        {
            let mut ops = BytesRefSelector::new(&mut values);
            RadixSelector::new(7).select(&mut ops, 0, 5, 2);
        }
        assert_eq!(values[2].slice(), b"charlie");
        for value in &values[..2] {
            assert!(value.slice() < b"charlie".as_slice());
        }
        for value in &values[3..] {
            assert!(value.slice() > b"charlie".as_slice());
        }
    }

    #[test]
    fn radix_select_handles_a_fully_shared_prefix() {
        // Every element identical: `computeCommonPrefixLength` consumes the
        // whole string and the recursion must stop rather than spin.
        let mut values: Vec<BytesRef> = (0..200).map(|_| BytesRef::new(b"same".to_vec())).collect();
        {
            let mut ops = BytesRefSelector::new(&mut values);
            RadixSelector::new(4).select(&mut ops, 0, 200, 100);
        }
        assert!(values.iter().all(|value| value.slice() == b"same"));
    }

    /// The arrangements Lucene 10.5.0's own `IntroSorter`, `IntroSelector` and
    /// `RadixSelector` leave, captured by running them (`probe.SorterProbe`,
    /// an anonymous subclass of each over a fixed array).
    ///
    /// These are not "an ordering that satisfies the contract" — they are the
    /// exact permutation Lucene produces. That is what has to match: the BKD
    /// leaf writes the packed value of every point, and a leaf sorted with a
    /// comparator that ties writes whichever arrangement its algorithm left.
    /// A test that only checked the ordering contract would pass with the
    /// thresholds mutated, the three-way partition's equal-arm removed, or the
    /// ninther's `min`/`max` swapped, and still write different bytes.
    const GOLDEN: &str = include_str!("selector_golden.txt");

    fn golden_lines(prefix: &str) -> Vec<(Vec<String>, Vec<i64>)> {
        GOLDEN
            .lines()
            .filter(|line| line.starts_with(prefix))
            .map(|line| {
                let mut parts = line.split_whitespace();
                let tags: Vec<String> = parts
                    .by_ref()
                    .take_while(|token| token.contains('=') || !token.contains(','))
                    .map(str::to_string)
                    .collect();
                let payload = line.rsplit_once(' ').expect("a payload").1;
                let values = payload
                    .split(',')
                    .map(|value| value.parse::<i64>().expect("an integer"))
                    .collect();
                (tags, values)
            })
            .collect()
    }

    fn tag(tags: &[String], key: &str) -> i64 {
        tags.iter()
            .find_map(|token| token.strip_prefix(key))
            .unwrap_or_else(|| panic!("missing {key} in {tags:?}"))
            .parse()
            .expect("a number")
    }

    fn golden_input(n: i64, modulus: i64) -> Vec<i32> {
        golden_lines("sort_in ")
            .into_iter()
            .find(|(tags, _)| tag(tags, "n=") == n && tag(tags, "mod=") == modulus)
            .map(|(_, values)| values.into_iter().map(|v| v as i32).collect())
            .unwrap_or_else(|| panic!("no golden input for n={n} mod={modulus}"))
    }

    #[test]
    fn intro_sort_leaves_the_arrangement_lucene_leaves() {
        let mut checked = 0;
        for (tags, expected) in golden_lines("sort_out ") {
            let (n, modulus) = (tag(&tags, "n="), tag(&tags, "mod="));
            let mut values = golden_input(n, modulus);
            {
                let mut ops = IntSelector {
                    values: &mut values,
                    pivot: 0,
                };
                intro_sort(&mut ops, 0, n as usize);
            }
            let expected: Vec<i32> = expected.into_iter().map(|v| v as i32).collect();
            assert_eq!(
                values, expected,
                "n={n} mod={modulus}: the arrangement must be Lucene's, not merely sorted"
            );
            checked += 1;
        }
        assert!(checked >= 6, "the golden file must carry sort vectors");
    }

    #[test]
    fn intro_select_leaves_the_arrangement_lucene_leaves() {
        let mut checked = 0;
        for (tags, expected) in golden_lines("select_out ") {
            let (n, modulus, k) = (
                tag(&tags, "n="),
                tag(&tags, "mod="),
                tag(&tags, "k=") as usize,
            );
            let mut values = golden_input(n, modulus);
            {
                let mut ops = IntSelector {
                    values: &mut values,
                    pivot: 0,
                };
                intro_select(&mut ops, 0, n as usize, k);
            }
            let expected: Vec<i32> = expected.into_iter().map(|v| v as i32).collect();
            assert_eq!(
                values, expected,
                "n={n} mod={modulus} k={k}: the arrangement must be Lucene's"
            );
            checked += 1;
        }
        assert!(
            checked >= 12,
            "the golden file must carry selection vectors"
        );
    }

    fn golden_strings(prefix: &str) -> Vec<Vec<BytesRef>> {
        GOLDEN
            .lines()
            .filter(|line| line.starts_with(prefix))
            .map(|line| {
                let payload = line.rsplit_once(' ').expect("a payload").1;
                payload
                    .split(',')
                    .map(|hex| {
                        BytesRef::new(
                            (0..hex.len() / 2)
                                .map(|i| {
                                    u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                                        .expect("two hex digits")
                                })
                                .collect(),
                        )
                    })
                    .collect()
            })
            .collect()
    }

    fn assert_radix_matches_golden(in_prefix: &str, out_prefix: &str, max_length: usize) {
        let inputs = golden_strings(in_prefix);
        let outputs = golden_strings(out_prefix);
        assert_eq!(inputs.len(), outputs.len());
        assert!(
            !inputs.is_empty(),
            "the golden file must carry {in_prefix} vectors"
        );
        for (input, expected) in inputs.into_iter().zip(outputs) {
            let count = input.len();
            let k = count / 2;
            let mut values = input;
            {
                let mut ops = BytesRefSelector::with_key_length(&mut values, max_length);
                RadixSelector::new(max_length).select(&mut ops, 0, count, k);
            }
            let actual: Vec<&[u8]> = values.iter().map(|v| v.slice()).collect();
            let expected: Vec<&[u8]> = expected.iter().map(|v| v.slice()).collect();
            assert_eq!(
                actual, expected,
                "n={count} k={k}: the arrangement must be Lucene's, bucket by bucket"
            );
        }
    }

    #[test]
    fn radix_select_leaves_the_arrangement_lucene_leaves() {
        // 14-byte strings whose first varying byte takes three values, so the
        // top-level buckets straddle `LENGTH_THRESHOLD` and the boundary
        // between radix and fallback is actually crossed.
        assert_radix_matches_golden("radix_in ", "radix_out ", 14);
    }

    #[test]
    fn radix_select_recurses_to_lucenes_depth() {
        // Four-byte strings over a three-value alphabet, 1200 of them: the
        // range is still above `LENGTH_THRESHOLD` after two bucket splits, so
        // the third level is where `LEVEL_THRESHOLD` decides between radix and
        // fallback. No shallower input can tell the two apart.
        assert_radix_matches_golden("deep_in ", "deep_out ", 4);
    }

    #[test]
    fn log2_matches_the_java_helper() {
        // `MathUtil.log(x, 2)` answers 0 for a non-positive argument and the
        // index of the highest set bit otherwise.
        assert_eq!(log2(0), 0);
        assert_eq!(log2(1), 0);
        assert_eq!(log2(2), 1);
        assert_eq!(log2(3), 1);
        assert_eq!(log2(4), 2);
        assert_eq!(log2(1023), 9);
        assert_eq!(log2(1024), 10);
    }
}
