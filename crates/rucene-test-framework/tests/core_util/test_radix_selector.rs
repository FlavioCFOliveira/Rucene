//! Port of `lucene/core/src/test/org/apache/lucene/util/TestRadixSelector.java`.
//!
//! The `BytesRef[]` arrays hold shared references (`Arc<BytesRef>`), so that
//! `arr.clone()` copies references and `sameInstance` is `Arc::ptr_eq`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::selector::{RadixSelector, RadixSelectorOps};
use rucene::util::{ArrayUtil, BytesRef};
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};

/// `TestRadixSelector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestRadixSelector",
        vec![
            test("testSelect", test_select),
            test("testSharedPrefixes", test_shared_prefixes),
        ],
    )
}

fn test_select() -> Result<()> {
    for _ in 0..100 {
        do_test_select();
    }
    Ok(())
}

/// The random `BytesRef[]` of `doTestSelect` and `doTestSharedPrefixes`.
fn random_array(from: i32, to: i32, max_len: i32) -> Vec<BytesRef> {
    let n = from + to + random().next_int_bound(5);
    let mut arr = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let mut bytes = vec![0u8; TestUtil::next_int(&*random(), 0, max_len) as usize];
        random().next_bytes(&mut bytes);
        arr.push(BytesRef::new(bytes));
    }
    arr
}

fn do_test_select() {
    let from = random().next_int_bound(5);
    let to = from + TestUtil::next_int(&*random(), 1, 10000);
    let max_len = TestUtil::next_int(&*random(), 1, 12);
    let arr = random_array(from, to, max_len);
    do_test(
        arr.into_iter().map(Arc::new).collect(),
        from as usize,
        to as usize,
        max_len,
    );
}

fn test_shared_prefixes() -> Result<()> {
    for _ in 0..100 {
        do_test_shared_prefixes();
    }
    Ok(())
}

fn do_test_shared_prefixes() {
    let from = random().next_int_bound(5);
    let to = from + TestUtil::next_int(&*random(), 1, 10000);
    let max_len = TestUtil::next_int(&*random(), 1, 12);
    let mut arr = random_array(from, to, max_len);
    let shared_prefix_length =
        (arr[0].length as i32).min(TestUtil::next_int(&*random(), 1, max_len)) as usize;
    for i in 1..arr.len() {
        let n = shared_prefix_length.min(arr[i].length);
        let (src_off, dst_off) = (arr[0].offset, arr[i].offset);
        let prefix = arr[0].bytes[src_off..src_off + n].to_vec();
        arr[i].bytes[dst_off..dst_off + n].copy_from_slice(&prefix);
    }
    do_test(
        arr.into_iter().map(Arc::new).collect(),
        from as usize,
        to as usize,
        max_len,
    );
}

/// The anonymous `RadixSelector` of `doTest`.
struct Selector<'a> {
    actual: &'a mut [Arc<BytesRef>],
    enforced_max_len: i32,
}

impl RadixSelectorOps for Selector<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        ArrayUtil::swap(self.actual, i, j);
    }

    fn byte_at(&self, i: usize, k: usize) -> i32 {
        Assert::assert_true((k as i64) < self.enforced_max_len as i64);
        let b = &self.actual[i];
        if k >= b.length {
            -1
        } else {
            b.bytes[b.offset + k] as i32
        }
    }
}

fn do_test(arr: Vec<Arc<BytesRef>>, from: usize, to: usize, max_len: i32) {
    let k = TestUtil::next_int(&*random(), from as i32, to as i32 - 1) as usize;
    let mut expected = arr.clone();
    // `Arrays.sort(Object[], from, to)`: a stable sort over `compareTo`.
    expected[from..to].sort();
    let mut actual = arr.clone();
    let enforced_max_len = if random().next_boolean() {
        max_len
    } else {
        i32::MAX
    };
    let mut selector = RadixSelector::new(enforced_max_len as usize);
    selector.select(
        &mut Selector {
            actual: &mut actual,
            enforced_max_len,
        },
        from,
        to,
        k,
    );

    // assertThat(actual[k], equalTo(expected[k]))
    assert_equals(&*expected[k], &*actual[k]);
    for i in 0..actual.len() {
        if i < from || i >= to {
            // assertThat(actual[i], sameInstance(arr[i]))
            Assert::assert_true(Arc::ptr_eq(&actual[i], &arr[i]));
        } else if i <= k {
            // assertThat(actual[i], lessThanOrEqualTo(actual[k]))
            Assert::assert_true(*actual[i] <= *actual[k]);
        } else {
            // assertThat(actual[i], greaterThanOrEqualTo(actual[k]))
            Assert::assert_true(*actual[i] >= *actual[k]);
        }
    }
}
