//! Port of
//! `lucene/core/src/test/org/apache/lucene/util/TestTimSorterWorstCase.java`
//! (`@Nightly`).

use std::collections::VecDeque;
use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::packed::{PackedInts, PackedIntsMutable};
use rucene::util::selector::PivotOps;
use rucene::util::sorter::{tim_min_run, Sorter, TimSorter, TimSorterState};
use rucene_test_framework::util::lucene_test_case::{Nightly, TEST_NIGHTLY};

use crate::core_support::{class_annotated, random, test};

/// `TestTimSorterWorstCase` (`@Nightly`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.util.TestTimSorterWorstCase",
        vec![test("testWorstCaseStackSize", test_worst_case_stack_size)],
        |b| b.annotate(Nightly),
    )
}

/// The anonymous `TimSorter(0)` of `testWorstCaseStackSize`.
struct WorstCaseSorter {
    arr: Box<dyn PackedIntsMutable>,
    state: TimSorterState,
    /// `Sorter.pivotIndex`, set by Java's default `setPivot`.
    pivot_index: usize,
}

fn unsupported() -> ! {
    Throwable::new("java.lang.UnsupportedOperationException", String::new()).throw()
}

impl PivotOps for WorstCaseSorter {
    fn swap(&mut self, i: usize, j: usize) {
        let tmp = self.arr.get(i as i32);
        let vj = self.arr.get(j as i32);
        self.arr.set(i as i32, vj);
        self.arr.set(j as i32, tmp);
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        // Long.compare
        match self.arr.get(i as i32).cmp(&self.arr.get(j as i32)) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    // `Sorter.setPivot`/`comparePivot`: Java's defaults over `pivotIndex`.
    fn set_pivot(&mut self, i: usize) {
        self.pivot_index = i;
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        let p = self.pivot_index;
        self.compare(p, j)
    }
}

impl Sorter for WorstCaseSorter {
    fn sort(&mut self, from: usize, to: usize) {
        self.tim_sort(from, to);
    }

    fn do_rotate(&mut self, lo: usize, mid: usize, hi: usize) {
        self.tim_do_rotate(lo, mid, hi);
    }
}

impl TimSorter for WorstCaseSorter {
    fn tim_state(&mut self) -> &mut TimSorterState {
        &mut self.state
    }

    fn copy(&mut self, src: usize, dest: usize) {
        let v = self.arr.get(src as i32);
        self.arr.set(dest as i32, v);
    }

    fn save(&mut self, _i: usize, _len: usize) {
        unsupported()
    }

    fn restore(&mut self, _i: usize, _j: usize) {
        unsupported()
    }

    fn compare_saved(&mut self, _i: usize, _j: usize) -> i32 {
        unsupported()
    }
}

fn test_worst_case_stack_size() -> Result<()> {
    // we need large arrays to be able to reproduce this bug
    // but not so big we blow up available heap.
    let length = if *TEST_NIGHTLY {
        RandomNumbers::random_int_between(&*random(), 140000000, 400000000)
    } else {
        RandomNumbers::random_int_between(&*random(), 140000000, 200000000)
    };
    let arr = generate_worst_case_array(length)?;
    WorstCaseSorter {
        arr,
        state: TimSorterState::new(0),
        pivot_index: 0,
    }
    .sort(0, length as usize);
    Ok(())
}

/// Create an array for the given list of runs.
fn create_array(length: i32, runs: &VecDeque<i32>) -> Result<Box<dyn PackedIntsMutable>> {
    let mut array = PackedInts::get_mutable(length, 1, 0.0)?;
    let mut end_run: i64 = -1;
    for &len in runs {
        end_run += len as i64;
        array.set(end_run as i32, 1);
    }
    array.set(length - 1, 0);
    Ok(array)
}

/// Create an array that triggers a worst-case sequence of run lens.
pub fn generate_worst_case_array(length: i32) -> Result<Box<dyn PackedIntsMutable>> {
    let min_run = tim_min_run(length as usize) as i32;
    let runs = runs_worst_case(length, min_run);
    create_array(length, &runs)
}

/// Fills `runs` with a sequence of run lengths of the form
/// `Y_n x_{n,1} x_{n,2} ... x_{n,l_n}`, …, `Y_1 x_{1,1} x_{1,2} ... x_{1,l_1}`.
/// The `Y_i`'s are chosen to satisfy the invariant throughout execution, but
/// the `x_{i,j}`'s are merged (by `TimSort.mergeCollapse`) into an `X_i` that
/// violates the invariant.
#[allow(non_snake_case)]
fn runs_worst_case(length: i32, min_run: i32) -> VecDeque<i32> {
    let mut runs: VecDeque<i32> = VecDeque::new();

    let mut running_total: i32 = 0;
    let mut Y = min_run + 4;
    let mut X = min_run;

    while running_total as i64 + Y as i64 + X as i64 <= length as i64 {
        running_total += X + Y;
        generate_wrong_elem(X, min_run, &mut runs);
        runs.push_front(Y);

        // X_{i+1} = Y_i + x_{i,1} + 1, since runs.get(1) = x_{i,1}
        X = Y + runs[1] + 1;

        // Y_{i+1} = X_{i+1} + Y_i + 1
        Y += X + 1;
    }

    if running_total as i64 + X as i64 <= length as i64 {
        running_total += X;
        generate_wrong_elem(X, min_run, &mut runs);
    }

    runs.push_back(length - running_total);
    runs
}

/// Adds a sequence `x_1, ..., x_n` of run lengths to `runs` such that:
/// 1. `X = x_1 + ... + x_n`;
/// 2. `x_j >= minRun` for all `j`;
/// 3. `x_1 + ... + x_{j-2} < x_j < x_1 + ... + x_{j-1}` for all `j`.
///
/// These conditions guarantee that TimSort merges all `x_j`'s one by one
/// (resulting in `X`) using only merges on the second-to-last element.
#[allow(non_snake_case)]
fn generate_wrong_elem(mut X: i32, min_run: i32, runs: &mut VecDeque<i32>) {
    while X >= 2 * min_run + 1 {
        // Default strategy
        let mut new_total = X / 2 + 1;

        // Specialized strategies
        if 3 * min_run + 3 <= X && X <= 4 * min_run + 1 {
            // add x_1=MIN+1, x_2=MIN, x_3=X-newTotal  to runs
            new_total = 2 * min_run + 1;
        } else if 5 * min_run + 5 <= X && X <= 6 * min_run + 5 {
            // add x_1=MIN+1, x_2=MIN, x_3=MIN+2, x_4=X-newTotal  to runs
            new_total = 3 * min_run + 3;
        } else if 8 * min_run + 9 <= X && X <= 10 * min_run + 9 {
            // add x_1=MIN+1, x_2=MIN, x_3=MIN+2, x_4=2MIN+2, x_5=X-newTotal  to runs
            new_total = 5 * min_run + 5;
        } else if 13 * min_run + 15 <= X && X <= 16 * min_run + 17 {
            // add x_1=MIN+1, x_2=MIN, x_3=MIN+2, x_4=2MIN+2, x_5=3MIN+4,
            // x_6=X-newTotal  to runs
            new_total = 8 * min_run + 9;
        }
        runs.push_front(X - new_total);
        X = new_total;
    }
    runs.push_front(X);
}
