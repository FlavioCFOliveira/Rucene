//! Port of `lucene/core/src/test/org/apache/lucene/util/TestMergedIterator.java`.

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::extra::MergedIterator;
use rucene::util::ArrayUtil;

use crate::core_support::{assert_equals, class, random, test};

const REPEATS: i32 = 2;
const VALS_TO_MERGE: i32 = 15000;

/// `@Repeat(iterations = REPEATS)` on a test method.
fn repeated(name: &str, body: fn() -> Result<()>) -> Method {
    test(name, body).annotate(Repeat {
        iterations: REPEATS,
        use_constant_seed: false,
    })
}

/// `TestMergedIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestMergedIterator",
        vec![
            test("testMergeEmpty", test_merge_empty),
            repeated("testNoDupsRemoveDups", || test_case(1, 1, true)),
            repeated("testOffItrDupsRemoveDups", || test_case(3, 1, true)),
            repeated("testOnItrDupsRemoveDups", || test_case(1, 3, true)),
            repeated("testOnItrRandomDupsRemoveDups", || test_case(1, -3, true)),
            repeated("testBothDupsRemoveDups", || test_case(3, 3, true)),
            repeated("testBothDupsWithRandomDupsRemoveDups", || {
                test_case(3, -3, true)
            }),
            repeated("testNoDupsKeepDups", || test_case(1, 1, false)),
            repeated("testOffItrDupsKeepDups", || test_case(3, 1, false)),
            repeated("testOnItrDupsKeepDups", || test_case(1, 3, false)),
            repeated("testOnItrRandomDupsKeepDups", || test_case(1, -3, false)),
            repeated("testBothDupsKeepDups", || test_case(3, 3, false)),
            repeated("testBothDupsWithRandomDupsKeepDups", || {
                test_case(3, -3, false)
            }),
        ],
    )
}

type BoxedIter = Box<dyn Iterator<Item = i32>>;

fn test_merge_empty() -> Result<()> {
    // Java's `hasNext()` is `peek().is_some()` over a peekable iterator.
    let mut merged = MergedIterator::new(Vec::<BoxedIter>::new()).peekable();
    Assert::assert_false(merged.peek().is_some());

    let mut merged =
        MergedIterator::new(vec![Box::new(Vec::<i32>::new().into_iter()) as BoxedIter]).peekable();
    Assert::assert_false(merged.peek().is_some());

    let n = random().next_int_bound(100);
    let mut itrs: Vec<BoxedIter> = Vec::with_capacity(n as usize);
    for _ in 0..n {
        itrs.push(Box::new(Vec::<i32>::new().into_iter()));
    }
    let mut merged = MergedIterator::new(itrs).peekable();
    Assert::assert_false(merged.peek().is_some());
    Ok(())
}

/// `testCase(int itrsWithVal, int specifiedValsOnItr, boolean removeDups)`.
fn test_case(itrs_with_val: i32, specified_vals_on_itr: i32, remove_dups: bool) -> Result<()> {
    // Build a random number of lists
    let mut expected: Vec<i32> = Vec::new();
    let random = JdkRandom::new(random().next_long());
    let num_lists = itrs_with_val + random.next_int_bound(1000 - itrs_with_val);
    let mut lists: Vec<Vec<i32>> = vec![Vec::new(); num_lists as usize];
    let start = random.next_int_bound(1000000);
    let end = start + VALS_TO_MERGE / itrs_with_val / specified_vals_on_itr.abs();
    for i in start..end {
        let mut max_list = lists.len();
        let mut max_vals_on_itr = 0;
        let mut sum_vals_on_itr = 0;
        for _ in 0..itrs_with_val {
            let list = random.next_int_bound(max_list as i32) as usize;
            let vals_on_itr = if specified_vals_on_itr < 0 {
                1 + random.next_int_bound(-specified_vals_on_itr)
            } else {
                specified_vals_on_itr
            };
            max_vals_on_itr = max_vals_on_itr.max(vals_on_itr);
            sum_vals_on_itr += vals_on_itr;
            for _ in 0..vals_on_itr {
                lists[list].push(i);
            }
            max_list -= 1;
            ArrayUtil::swap(&mut lists, list, max_list);
        }
        let max_count = if remove_dups {
            max_vals_on_itr
        } else {
            sum_vals_on_itr
        };
        for _ in 0..max_count {
            expected.push(i);
        }
    }

    let itrs: Vec<BoxedIter> = lists
        .into_iter()
        .map(|l| Box::new(l.into_iter()) as BoxedIter)
        .collect();
    let mut merged_itr = MergedIterator::with_remove_duplicates(remove_dups, itrs).peekable();
    for e in expected {
        Assert::assert_true(merged_itr.peek().is_some());
        assert_equals(&Some(e), &merged_itr.next());
    }
    Assert::assert_false(merged_itr.peek().is_some());
    Ok(())
}
