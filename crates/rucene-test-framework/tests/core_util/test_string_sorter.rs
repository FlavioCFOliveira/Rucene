//! Port of `lucene/core/src/test/org/apache/lucene/util/TestStringSorter.java`.
//!
//! The anonymous `StringSorter.get(builder, result, i)` points `result` at
//! `refs[i]`; Rucene's [`StringSorterOps::get`] returns the `BytesRef`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::sorter::{
    NaturalBytesRefComparator, StableStringSorter, StableStringSorterOps, StringSorter,
    StringSorterComparator, StringSorterOps,
};
use rucene::util::BytesRef;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_array_equals, assert_equals, class, random, test};

/// `TestStringSorter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestStringSorter",
        vec![
            test("testEmpty", test_empty),
            test("testOneValue", test_one_value),
            test("testTwoValues", test_two_values),
            test("testRandom", test_random),
            test(
                "testRandomWithLotsOfDuplicates",
                test_random_with_lots_of_duplicates,
            ),
            test("testRandomWithSharedPrefix", test_random_with_shared_prefix),
            test(
                "testRandomWithSharedPrefixAndLotsOfDuplicates",
                test_random_with_shared_prefix_and_lots_of_duplicates,
            ),
        ],
    )
}

/// `Comparator.naturalOrder()` over `BytesRef`: `compareTo`.
fn natural_order(a: &BytesRef, b: &BytesRef) -> i32 {
    match a.cmp(b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// `test(BytesRef[] refs, int len)`.
fn test_refs(refs: &[Option<BytesRef>], len: usize) {
    let natural = NaturalBytesRefComparator;
    test_with(
        refs[0..len].to_vec(),
        len,
        StringSorterComparator::Radix(&natural),
    );
    test_with(
        refs[0..len].to_vec(),
        len,
        StringSorterComparator::Generic(&natural_order),
    );
    test_stable(
        refs[0..len].to_vec(),
        len,
        StringSorterComparator::Radix(&natural),
    );
    test_stable(
        refs[0..len].to_vec(),
        len,
        StringSorterComparator::Generic(&natural_order),
    );
}

/// `Arrays.sort(Object[])` over non-null `BytesRef`s.
fn sorted(refs: &[Option<BytesRef>]) -> Vec<Option<BytesRef>> {
    let mut expected = refs.to_vec();
    expected.sort_by(|a, b| {
        a.as_ref()
            .expect("non-null ref")
            .cmp(b.as_ref().expect("non-null ref"))
    });
    expected
}

struct Ops<'a> {
    refs: &'a mut Vec<Option<BytesRef>>,
}

impl StringSorterOps for Ops<'_> {
    fn get(&self, i: usize) -> BytesRef {
        self.refs[i].clone().expect("non-null ref")
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.refs.swap(i, j);
    }
}

/// `test(BytesRef[] refs, int len, Comparator<BytesRef> comparator)`.
fn test_with(mut refs: Vec<Option<BytesRef>>, len: usize, comparator: StringSorterComparator<'_>) {
    let expected = sorted(&refs[0..len]);
    StringSorter::new(comparator).sort(&mut Ops { refs: &mut refs }, 0, len);
    let actual = refs[0..len].to_vec();
    assert_array_equals(&expected, &actual);
}

struct StableOps<'a> {
    refs: &'a [Option<BytesRef>],
    ord: &'a mut Vec<usize>,
    tmp: Vec<usize>,
}

impl StringSorterOps for StableOps<'_> {
    fn get(&self, i: usize) -> BytesRef {
        self.refs[self.ord[i]].clone().expect("non-null ref")
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.ord.swap(i, j);
    }
}

impl StableStringSorterOps for StableOps<'_> {
    fn save(&mut self, i: usize, j: usize) {
        self.tmp[j] = self.ord[i];
    }

    fn restore(&mut self, i: usize, j: usize) {
        self.ord[i..j].copy_from_slice(&self.tmp[i..j]);
    }
}

/// `testStable(BytesRef[] refs, int len, Comparator<BytesRef> comparator)`.
fn test_stable(refs: Vec<Option<BytesRef>>, len: usize, comparator: StringSorterComparator<'_>) {
    let expected = sorted(&refs[0..len]);
    let mut ord: Vec<usize> = (0..len).collect();
    StableStringSorter::new(comparator).sort(
        &mut StableOps {
            refs: &refs,
            ord: &mut ord,
            tmp: vec![0; len],
        },
        0,
        len,
    );

    for i in 0..len {
        assert_equals(&expected[i], &refs[ord[i]]);
        if i > 0 && expected[i] == expected[i - 1] {
            Assert::assert_true_msg(
                &format!("not stable: {} <= {}", ord[i], ord[i - 1]),
                ord[i] > ord[i - 1],
            );
        }
    }
}

fn test_empty() -> Result<()> {
    test_refs(&vec![None; random().next_int_bound(5) as usize], 0);
    Ok(())
}

fn test_one_value() -> Result<()> {
    let bytes = BytesRef::from_char_sequence(&TestUtil::random_simple_string(&*random()));
    test_refs(&[Some(bytes)], 1);
    Ok(())
}

fn test_two_values() -> Result<()> {
    let bytes1 = BytesRef::from_char_sequence(&TestUtil::random_simple_string(&*random()));
    let bytes2 = BytesRef::from_char_sequence(&TestUtil::random_simple_string(&*random()));
    test_refs(&[Some(bytes1), Some(bytes2)], 2);
    Ok(())
}

/// `testRandom(int commonPrefixLen, int maxLen)`.
fn test_random_with(common_prefix_len: i32, max_len: i32) {
    let mut common_prefix = vec![0u8; common_prefix_len as usize];
    random().next_bytes(&mut common_prefix);
    let len = random().next_int_bound(100000) as usize;
    let mut bytes: Vec<Option<BytesRef>> = vec![None; len + random().next_int_bound(50) as usize];
    for i in 0..len {
        let mut b = vec![0u8; (common_prefix_len + random().next_int_bound(max_len)) as usize];
        random().next_bytes(&mut b);
        b[..common_prefix_len as usize].copy_from_slice(&common_prefix);
        bytes[i] = Some(BytesRef::new(b));
    }
    test_refs(&bytes, len);
}

fn test_random() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(3);
    for _ in 0..num_iters {
        test_random_with(0, 10);
    }
    Ok(())
}

fn test_random_with_lots_of_duplicates() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(3);
    for _ in 0..num_iters {
        test_random_with(0, 2);
    }
    Ok(())
}

fn test_random_with_shared_prefix() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(3);
    for _ in 0..num_iters {
        test_random_with(TestUtil::next_int(&*random(), 1, 30), 10);
    }
    Ok(())
}

fn test_random_with_shared_prefix_and_lots_of_duplicates() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(3);
    for _ in 0..num_iters {
        test_random_with(TestUtil::next_int(&*random(), 1, 30), 2);
    }
    Ok(())
}
