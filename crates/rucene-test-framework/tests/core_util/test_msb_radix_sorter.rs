//! Port of `lucene/core/src/test/org/apache/lucene/util/TestMSBRadixSorter.java`
//! and of `TestStableMSBRadixSorter.java`, which differs only in sorting with
//! a `StableMSBRadixSorter` (its `save`/`restore` over a `temp` array) and in
//! checking that every slot holds the very same instance `Arrays.sort` put
//! there.
//!
//! The `BytesRef[]` arrays hold shared references (`Option<Arc<BytesRef>>`,
//! `None` for Java's `null` slots), so that copies copy references and
//! `assertSame` is `Arc::ptr_eq`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::sorter::{
    MSBRadixSorter, MSBRadixSorterOps, StableMSBRadixSorter, StableMSBRadixSorterOps,
};
use rucene::util::{BytesRef, BytesRefBuilder};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_array_equals, assert_equals, class, random, JavaHashSet};
use crate::test_with;

type Refs = Vec<Option<Arc<BytesRef>>>;

/// `TestMSBRadixSorter`.
pub fn class_def() -> Arc<Class> {
    class("org.apache.lucene.util.TestMSBRadixSorter", methods(false))
}

/// `TestStableMSBRadixSorter`.
pub fn stable_class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestStableMSBRadixSorter",
        methods(true),
    )
}

fn methods(stable: bool) -> Vec<randomizedtesting::jdk::reflect::Method> {
    vec![
        test_with("testEmpty", move || test_empty(stable)),
        test_with("testOneValue", move || test_one_value(stable)),
        test_with("testTwoValues", move || test_two_values(stable)),
        test_with("testRandom", move || {
            for _ in 0..10 {
                test_random_with(stable, 0, 10);
            }
            Ok(())
        }),
        test_with("testRandomWithLotsOfDuplicates", move || {
            for _ in 0..10 {
                test_random_with(stable, 0, 2);
            }
            Ok(())
        }),
        test_with("testRandomWithSharedPrefix", move || {
            for _ in 0..10 {
                test_random_with(stable, TestUtil::next_int(&*random(), 1, 30), 10);
            }
            Ok(())
        }),
        test_with("testRandomWithSharedPrefixAndLotsOfDuplicates", move || {
            for _ in 0..10 {
                test_random_with(stable, TestUtil::next_int(&*random(), 1, 30), 2);
            }
            Ok(())
        }),
        test_with("testRandom2", move || test_random2(stable)),
    ]
}

/// The anonymous `MSBRadixSorter` (and `StableMSBRadixSorter`) of `test`.
struct Ops<'a> {
    refs: &'a mut Refs,
    final_max_length: i32,
    temp: Option<Refs>,
}

impl MSBRadixSorterOps for Ops<'_> {
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        Assert::assert_true((k as i64) < self.final_max_length as i64);
        let r = self.refs[i].as_ref().expect("non-null ref");
        if r.length <= k {
            return -1;
        }
        r.bytes[r.offset + k] as i32
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.refs.swap(i, j);
    }
}

impl StableMSBRadixSorterOps for Ops<'_> {
    fn save(&mut self, i: usize, j: usize) {
        let len = self.refs.len();
        let temp = self.temp.get_or_insert_with(|| vec![None; len]);
        temp[j] = self.refs[i].clone();
    }

    fn restore(&mut self, i: usize, j: usize) {
        if let Some(temp) = &self.temp {
            self.refs[i..j].clone_from_slice(&temp[i..j]);
        }
    }
}

/// `test(BytesRef[] refs, int len)`.
fn test_refs(stable: bool, mut refs: Refs, len: usize) {
    let mut expected: Refs = refs[0..len].to_vec();
    // `Arrays.sort(Object[])`: a stable sort over `compareTo`.
    expected.sort_by(|a, b| {
        a.as_ref()
            .expect("non-null ref")
            .cmp(b.as_ref().expect("non-null ref"))
    });

    let mut max_length: i32 = 0;
    for i in 0..len {
        let r = refs[i].as_ref().expect("non-null ref");
        max_length = max_length.max(r.length as i32);
    }
    match random().next_int_bound(3) {
        0 => max_length += TestUtil::next_int(&*random(), 1, 5),
        1 => max_length = i32::MAX,
        _ => {
            // leave unchanged
        }
    }

    let final_max_length = max_length;
    {
        let mut ops = Ops {
            refs: &mut refs,
            final_max_length,
            temp: None,
        };
        if stable {
            StableMSBRadixSorter::new(max_length as usize).sort(&mut ops, 0, len);
        } else {
            MSBRadixSorter::new(max_length as usize).sort(&mut ops, 0, len);
        }
    }
    let actual: Refs = refs[0..len].to_vec();
    let expected_values: Vec<Option<&BytesRef>> = expected.iter().map(|r| r.as_deref()).collect();
    let actual_values: Vec<Option<&BytesRef>> = actual.iter().map(|r| r.as_deref()).collect();
    assert_array_equals(&expected_values, &actual_values);

    if stable {
        // Verify that the arrays are not only equal after sorting with
        // Arrays#sort and StableMSBRadixSorter but also that they have the
        // very same instance at every index. This is different from
        // MSBRadixSorter which does not guarantee ordering of the same value.
        assert_equals(&expected.len(), &actual.len());
        for i in 0..expected.len() {
            // assertSame(expected[i].bytes, actual[i].bytes)
            Assert::assert_true(Arc::ptr_eq(
                expected[i].as_ref().expect("non-null ref"),
                actual[i].as_ref().expect("non-null ref"),
            ));
        }
    }
}

fn test_empty(stable: bool) -> Result<()> {
    test_refs(stable, vec![None; random().next_int_bound(5) as usize], 0);
    Ok(())
}

fn test_one_value(stable: bool) -> Result<()> {
    let bytes = BytesRef::from_char_sequence(&TestUtil::random_simple_string(&*random()));
    test_refs(stable, vec![Some(Arc::new(bytes))], 1);
    Ok(())
}

fn test_two_values(stable: bool) -> Result<()> {
    let bytes1 = BytesRef::from_char_sequence(&TestUtil::random_simple_string(&*random()));
    let bytes2 = BytesRef::from_char_sequence(&TestUtil::random_simple_string(&*random()));
    test_refs(
        stable,
        vec![Some(Arc::new(bytes1)), Some(Arc::new(bytes2))],
        2,
    );
    Ok(())
}

/// `testRandom(int commonPrefixLen, int maxLen)`.
fn test_random_with(stable: bool, common_prefix_len: i32, max_len: i32) {
    let mut common_prefix = vec![0u8; common_prefix_len as usize];
    random().next_bytes(&mut common_prefix);
    let len = random().next_int_bound(100000) as usize;
    let mut bytes: Refs = vec![None; len + random().next_int_bound(50) as usize];
    for i in 0..len {
        let mut b = vec![0u8; (common_prefix_len + random().next_int_bound(max_len)) as usize];
        random().next_bytes(&mut b);
        b[..common_prefix_len as usize].copy_from_slice(&common_prefix);
        bytes[i] = Some(Arc::new(BytesRef::new(b)));
    }
    test_refs(stable, bytes, len);
}

fn test_random2(stable: bool) -> Result<()> {
    // how large the alphabet is
    let letter_count = TestUtil::next_int(&*random(), 2, 10);

    // how many substring fragments to use
    let substring_count = TestUtil::next_int(&*random(), 2, 10);
    let mut substrings_set: JavaHashSet<BytesRef> = JavaHashSet::new();

    // how many strings to make
    let string_count = LuceneTestCase::at_least(10000);

    while substrings_set.size() < substring_count as usize {
        let length = TestUtil::next_int(&*random(), 2, 10);
        let mut bytes = vec![0u8; length as usize];
        for i in 0..length as usize {
            bytes[i] = random().next_int_bound(letter_count) as u8;
        }
        let br = BytesRef::new(bytes);
        substrings_set.add(br);
    }

    let substrings: Vec<BytesRef> = substrings_set.to_vec();
    let mut chance = vec![0f64; substrings.len()];
    let mut sum = 0.0;
    for i in 0..substrings.len() {
        chance[i] = random().next_double();
        sum += chance[i];
    }

    // give each substring a random chance of occurring:
    let mut accum = 0.0;
    for i in 0..substrings.len() {
        accum += chance[i] / sum;
        chance[i] = accum;
    }

    let mut strings_set: JavaHashSet<BytesRef> = JavaHashSet::new();
    let mut iters = 0;
    while strings_set.size() < string_count as usize && iters < string_count * 5 {
        let count = TestUtil::next_int(&*random(), 1, 5);
        let mut b = BytesRefBuilder::new();
        for _ in 0..count {
            let v = random().next_double();
            accum = 0.0;
            for j in 0..substrings.len() {
                accum += chance[j];
                if accum >= v {
                    b.append_ref(&substrings[j]);
                    break;
                }
            }
        }
        let br = b.to_bytes_ref();
        strings_set.add(br);
        iters += 1;
    }

    let refs: Refs = strings_set
        .to_vec()
        .into_iter()
        .map(|r| Some(Arc::new(r)))
        .collect();
    let len = refs.len();
    test_refs(stable, refs, len);
    Ok(())
}
