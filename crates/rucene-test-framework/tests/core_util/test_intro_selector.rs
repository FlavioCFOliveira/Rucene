//! Port of `lucene/core/src/test/org/apache/lucene/util/TestIntroSelector.java`.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::selector::{intro_select, intro_select_with_max_depth, PivotOps};
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};

/// `TestIntroSelector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestIntroSelector",
        vec![test("testSelect", test_select)],
    )
}

fn test_select() -> Result<()> {
    let random = random();
    for _ in 0..100 {
        do_test_select(&*random);
    }
    Ok(())
}

/// The anonymous `IntroSelector` of `doTestSelect`, over `actual`.
struct Selector<'a> {
    actual: &'a mut [i32],
    pivot: i32,
}

impl PivotOps for Selector<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.actual.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot = self.actual[i];
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.pivot.cmp(&self.actual[j]) as i32
    }
}

/// `doTestSelect(random)`.
fn do_test_select(random: &dyn Random) {
    let from = random.next_int_bound(5);
    let to = from + TestUtil::next_int(random, 1, 10000);
    let max = if random.next_boolean() {
        random.next_int_bound(100)
    } else {
        random.next_int_bound(100000)
    };
    let mut arr: Vec<i32> = vec![0; (to + random.next_int_bound(5)) as usize];
    for i in 0..arr.len() {
        arr[i] = TestUtil::next_int(random, 0, max);
    }
    let k = TestUtil::next_int(random, from, to - 1);

    let mut expected = arr.clone();
    expected[from as usize..to as usize].sort();

    let mut actual = arr.clone();
    {
        let mut selector = Selector {
            actual: &mut actual,
            pivot: 0,
        };
        if random.next_boolean() {
            intro_select(&mut selector, from as usize, to as usize, k as usize);
        } else {
            intro_select_with_max_depth(
                &mut selector,
                from as usize,
                to as usize,
                k as usize,
                random.next_int_bound(3),
            );
        }
    }

    let k = k as usize;
    assert_equals(&expected[k], &actual[k]);
    for i in 0..actual.len() {
        if i < from as usize || i >= to as usize {
            // `assertSame(arr[i], actual[i])`: the same boxed `Integer`.
            assert_equals(&arr[i], &actual[i]);
        } else if i <= k {
            Assert::assert_true(actual[i] <= actual[k]);
        } else {
            Assert::assert_true(actual[i] >= actual[k]);
        }
    }
}
