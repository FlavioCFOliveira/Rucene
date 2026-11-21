//! Port of `lucene/core/src/test/org/apache/lucene/util/TestFixedLengthBytesRefArray.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::{
    BytesRef, BytesRefIterator, FixedLengthBytesRefArray, SortableBytesRefArray,
    StringSorterComparator,
};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_not_null, class, random, test};

/// `TestFixedLengthBytesRefArray`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestFixedLengthBytesRefArray",
        vec![
            test("testBasic", test_basic),
            test("testRandom", test_random),
        ],
    )
}

/// `BytesRef::compareTo`.
fn compare_to(a: &BytesRef, b: &BytesRef) -> i32 {
    a.cmp(b) as i32
}

fn test_basic() -> Result<()> {
    let mut a = FixedLengthBytesRefArray::new(4); // Integer.BYTES
    let num_values = 100;
    for i in 0..num_values {
        let bytes: Vec<u8> = vec![0, 0, 0, (10 - i) as u8];
        a.append(&BytesRef::new(bytes))?;
    }

    let mut iterator = a.iterator(StringSorterComparator::Generic(&compare_to));
    let mut last: Option<BytesRef> = None;

    let mut count = 0;
    loop {
        let Some(bytes) = iterator.next()? else {
            break;
        };
        if let Some(last) = &last {
            Assert::assert_true_msg(
                &format!("count={count} last={last} bytes={bytes}"),
                compare_to(last, &bytes) < 0,
            );
        }
        last = Some(BytesRef::deep_copy_of(&bytes));
        count += 1;
    }

    assert_equals(&num_values, &count);
    Ok(())
}

fn test_random() -> Result<()> {
    let length = TestUtil::next_int(&*random(), 4, 10);
    let count = LuceneTestCase::at_least(10000);
    let mut values: Vec<BytesRef> = Vec::with_capacity(count as usize);

    let mut a = FixedLengthBytesRefArray::new(length as usize);
    for _ in 0..count {
        let mut value = BytesRef::new(vec![0u8; length as usize]);
        random().next_bytes(&mut value.bytes);
        a.append(&value)?;
        values.push(value);
    }

    for i in 0..count as usize {
        assert_equals(&values[i], &a.get(i)?);
    }

    values.sort();
    let mut iterator = a.iterator(StringSorterComparator::Generic(&compare_to));
    for i in 0..count as usize {
        let next = iterator.next()?;
        assert_not_null(&next);
        assert_equals(&values[i], next.as_ref().unwrap());
    }
    Ok(())
}
