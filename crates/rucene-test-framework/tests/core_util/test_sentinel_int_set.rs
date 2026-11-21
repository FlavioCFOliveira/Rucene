//! Port of `lucene/core/src/test/org/apache/lucene/util/TestSentinelIntSet.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::SentinelIntSet;

use crate::core_support::{assert_equals, class, random, test};

/// `TestSentinelIntSet`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestSentinelIntSet",
        vec![test("test", test_basic), test("testRandom", test_random)],
    )
}

fn test_basic() -> Result<()> {
    let mut set = SentinelIntSet::new(10, -1);
    Assert::assert_false(set.exists(50));
    set.put(50);
    Assert::assert_true(set.exists(50));
    assert_equals(&1, &set.size());
    assert_equals(&-11, &set.find(10));
    assert_equals(&1, &set.size());
    set.clear();
    assert_equals(&0, &set.size());
    assert_equals(&50, &set.hash(50));
    // force a rehash
    for i in 0..20 {
        set.put(i);
    }
    assert_equals(&20, &set.size());
    assert_equals(&24, &set.rehash_count);
    Ok(())
}

fn test_random() -> Result<()> {
    for _ in 0..10000 {
        let init_sz = random().next_int_bound(20);
        let num = random().next_int_bound(30);
        let max_val = (if random().next_boolean() {
            random().next_int_bound(50)
        } else {
            random().next_int_bound(i32::MAX)
        }) + 1;

        // Only `add` and `size` are used: iteration order is not observed.
        let mut a: HashSet<i32> = HashSet::with_capacity(init_sz as usize);
        let mut b = SentinelIntSet::new(init_sz as usize, -1);

        for _ in 0..num {
            let val = random().next_int_bound(max_val);
            let exists = !a.insert(val);
            let exists_b = b.exists(val);
            assert_equals(&exists, &exists_b);
            let slot = b.find(val);
            assert_equals(&exists, &(slot >= 0));
            b.put(val);

            assert_equals(&a.len(), &b.size());
        }
    }
    Ok(())
}
