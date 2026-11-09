//! Port of `lucene/core/src/test/org/apache/lucene/util/TestBytesRefArray.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::concurrent::Counter;
use rucene::util::{
    BytesRef, BytesRefArray, BytesRefBuilder, BytesRefIterator, IndexedBytesRefIterator,
    NaturalBytesRefComparator, SortableBytesRefArray, StringSorterComparator,
};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class, random, test, unchecked,
};

/// `TestBytesRefArray`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestBytesRefArray",
        vec![
            test("testAppend", test_append),
            test("testSort", test_sort),
            test("testStableSort", test_stable_sort),
        ],
    )
}

/// `Comparator.naturalOrder()` over `BytesRef`.
fn natural_order(a: &BytesRef, b: &BytesRef) -> i32 {
    a.cmp(b) as i32
}

/// `BytesRefComparator.NATURAL`.
static NATURAL: NaturalBytesRefComparator = NaturalBytesRefComparator;

/// `random().nextBoolean() ? Comparator.naturalOrder() :
/// BytesRefComparator.NATURAL`.
fn random_comparator() -> StringSorterComparator<'static> {
    if random().next_boolean() {
        StringSorterComparator::Generic(&natural_order)
    } else {
        StringSorterComparator::Radix(&NATURAL)
    }
}

/// `TestUtil.STRING_CODEPOINT_COMPARATOR`.
fn string_codepoint_comparator(a: &String, b: &String) -> std::cmp::Ordering {
    TestUtil::string_codepoint_compare(a, b).cmp(&0)
}

fn utf8(b: &BytesRef) -> String {
    unchecked(b.utf8_to_string())
}

fn test_append() -> Result<()> {
    let random = random();
    let mut list = BytesRefArray::new(Arc::new(Counter::new_counter()));
    let mut string_list: Vec<String> = Vec::new();
    for j in 0..2 {
        if j > 0 && random.next_boolean() {
            list.clear();
            string_list.clear();
        }
        let entries = LuceneTestCase::at_least(500);
        let mut spare = BytesRefBuilder::new();
        let init_size = list.size();
        for i in 0..entries as usize {
            let random_realistic_unicode_string =
                TestUtil::random_realistic_unicode_string(&*random);
            spare.copy_chars(&random_realistic_unicode_string);
            assert_equals(&(i + init_size), &list.append(&spare.get())?);
            string_list.push(random_realistic_unicode_string);
        }
        for i in 0..entries as usize {
            let got = Some(list.get(i)?);
            assert_not_null(&got);
            assert_equals_msg(
                Some(&format!("entry {i} doesn't match")),
                &string_list[i],
                &utf8(got.as_ref().unwrap()),
            );
        }

        // check random
        for i in 0..entries {
            let e = random.next_int_bound(entries) as usize;
            let got = Some(list.get(e)?);
            assert_not_null(&got);
            assert_equals_msg(
                Some(&format!("entry {i} doesn't match")),
                &string_list[e],
                &utf8(got.as_ref().unwrap()),
            );
        }
        for _ in 0..2 {
            let mut iterator = list.iterator();
            for string in &string_list {
                let next = iterator.next()?.expect("next");
                assert_equals(string, &utf8(&next));
            }
        }
    }
    Ok(())
}

fn test_sort() -> Result<()> {
    let random = random();
    let mut list = BytesRefArray::new(Arc::new(Counter::new_counter()));
    let mut string_list: Vec<String> = Vec::new();

    for j in 0..5 {
        if j > 0 && random.next_boolean() {
            list.clear();
            string_list.clear();
        }
        let entries = LuceneTestCase::at_least(200);
        let mut spare = BytesRefBuilder::new();
        let init_size = list.size();
        for i in 0..entries as usize {
            let random_realistic_unicode_string =
                TestUtil::random_realistic_unicode_string(&*random);
            spare.copy_chars(&random_realistic_unicode_string);
            assert_equals(&(init_size + i), &list.append(&spare.get())?);
            string_list.push(random_realistic_unicode_string);
        }

        string_list.sort_by(string_codepoint_comparator);
        let mut iter = list.sorted_iterator(random_comparator())?;
        let mut i = 0;
        while let Some(next) = iter.next()? {
            assert_equals_msg(
                Some(&format!("entry {i} doesn't match")),
                &string_list[i],
                &utf8(&next),
            );
            i += 1;
        }
        assert_null(&iter.next()?);
        assert_equals(&i, &string_list.len());
    }
    Ok(())
}

fn test_stable_sort() -> Result<()> {
    let random = random();
    let mut list = BytesRefArray::new(Arc::new(Counter::new_counter()));
    let mut string_list: Vec<String> = Vec::new();

    for j in 0..5 {
        if j > 0 && random.next_boolean() {
            list.clear();
            string_list.clear();
        }
        let entries = LuceneTestCase::at_least(200);
        let mut values: Vec<String> = Vec::with_capacity(20);
        for _ in 0..20 {
            values.push(TestUtil::random_realistic_unicode_string(&*random));
        }
        let mut spare = BytesRefBuilder::new();
        let init_size = list.size();
        for i in 0..entries as usize {
            let random_realistic_unicode_string =
                RandomPicks::random_from(&*random, &values).clone();
            spare.copy_chars(&random_realistic_unicode_string);
            assert_equals(&(init_size + i), &list.append(&spare.get())?);
            string_list.push(random_realistic_unicode_string);
        }

        string_list.sort_by(string_codepoint_comparator);
        let state = list.sort(random_comparator(), true);
        let mut iter = list.iterator_with_sort_state(Some(state));
        let mut i = 0;
        let mut last_ord: i64 = -1;
        let mut last: Option<BytesRef> = None;
        while let Some(next) = iter.next()? {
            assert_equals_msg(
                Some(&format!("entry {i} doesn't match")),
                &string_list[i],
                &utf8(&next),
            );
            i += 1;

            if last.as_ref() == Some(&next) {
                Assert::assert_true_msg(
                    &format!("sort not stable: {} <= {last_ord}", iter.ord()),
                    iter.ord() as i64 > last_ord,
                );
            }
            last = Some(BytesRef::deep_copy_of(&next));
            last_ord = iter.ord() as i64;
        }
        assert_null(&iter.next()?);
        assert_equals(&i, &string_list.len());
    }
    Ok(())
}
